import {
  GSplatProcessor,
  Mat4,
  type CameraComponent,
  type GSplatComponent,
  type GraphicsDevice,
  type Texture,
  WORKBUFFER_UPDATE_ONCE,
} from "playcanvas";
import type { GaussianCrop } from "../../types/pipeline";
import { getCurrentLocale, translate } from "../../i18n";
import { maskBit, packedMaskLength, setMaskBit } from "../../stores/gaussianTransformStore";

export type RectangleSelectionMode = "replace" | "add" | "remove";
export interface SelectionRectangle { minX: number; minY: number; maxX: number; maxY: number }

export const PROCESS_SELECTION_GLSL = /* glsl */ `
uniform mat4 uOoosplatViewProjection;
uniform mat4 uOoosplatModelMatrix;
uniform vec4 uOoosplatSelectionRect;
uniform float uOoosplatSelectionCropKind;
uniform vec3 uOoosplatSelectionCropCenter;
uniform vec3 uOoosplatSelectionCropSize;
uniform float uOoosplatSelectionCropRadius;
uniform float uOoosplatSelectionCropInverted;
uniform vec2 uOoosplatSelectionViewport;
uniform float uOoosplatSelectionOperation;

bool cropContains(vec3 center) {
    if (uOoosplatSelectionCropKind < 0.5) return true;
    bool inside = uOoosplatSelectionCropKind < 1.5
        ? distance(center, uOoosplatSelectionCropCenter) <= uOoosplatSelectionCropRadius
        : all(lessThanEqual(abs(center - uOoosplatSelectionCropCenter), uOoosplatSelectionCropSize * 0.5));
    return uOoosplatSelectionCropInverted > 0.5 ? !inside : inside;
}

vec3 ooosplatRotate(vec4 q, vec3 v) {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

vec2 ooosplatProjectedAxis(vec3 axis, vec4 clip) {
    vec4 delta = uOoosplatViewProjection * uOoosplatModelMatrix * vec4(axis, 0.0);
    return (delta.xy - clip.xy / clip.w * delta.w) / clip.w;
}

float ooosplatEllipseDistance(vec2 p, vec3 covariance) {
    float determinant = max(covariance.x * covariance.z - covariance.y * covariance.y, 1e-30);
    return (covariance.z * p.x * p.x - 2.0 * covariance.y * p.x * p.y + covariance.x * p.y * p.y) / determinant;
}

bool ooosplatEllipseHitsRect(vec2 center, vec3 cov, float limit) {
    vec2 lo = uOoosplatSelectionRect.xy - center;
    vec2 hi = uOoosplatSelectionRect.zw - center;
    if (all(lessThanEqual(lo, vec2(0.0))) && all(greaterThanEqual(hi, vec2(0.0)))) return true;
    float q0 = ooosplatEllipseDistance(vec2(lo.x, clamp(cov.y * lo.x / cov.x, lo.y, hi.y)), cov);
    float q1 = ooosplatEllipseDistance(vec2(hi.x, clamp(cov.y * hi.x / cov.x, lo.y, hi.y)), cov);
    float q2 = ooosplatEllipseDistance(vec2(clamp(cov.y * lo.y / cov.z, lo.x, hi.x), lo.y), cov);
    float q3 = ooosplatEllipseDistance(vec2(clamp(cov.y * hi.y / cov.z, lo.x, hi.x), hi.y), cov);
    return min(min(q0, q1), min(q2, q3)) <= limit;
}

bool ooosplatFootprintHits(vec4 clip) {
    if (clip.w <= 0.0 || abs(clip.z) > clip.w) return false;
    float opacity = getColor().a;
    if (loadOoosplatSelected().r > 0.5) opacity = max(opacity, 0.72);
    if (opacity < 1.0 / 255.0) return false;
    vec4 rotation = getRotation().yzwx;
    vec3 scale = getScale();
    vec2 x = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3(scale.x, 0.0, 0.0)), clip);
    vec2 y = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3(0.0, scale.y, 0.0)), clip);
    vec2 z = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3(0.0, 0.0, scale.z)), clip);
    // Match PlayCanvas' covariance blur and normalized, truncated Gaussian footprint.
    vec2 blur = 0.3 / max(uOoosplatSelectionViewport * uOoosplatSelectionViewport, vec2(1.0));
    vec3 cov = vec3(dot(vec3(x.x, y.x, z.x), vec3(x.x, y.x, z.x)) + blur.x,
                    dot(vec3(x.x, y.x, z.x), vec3(x.y, y.y, z.y)),
                    dot(vec3(x.y, y.y, z.y), vec3(x.y, y.y, z.y)) + blur.y);
    float limit = -2.0 * log(exp(-4.0) + (1.0 - exp(-4.0)) / (255.0 * opacity));
    return ooosplatEllipseHitsRect(clip.xy / clip.w, cov, limit);
}

void process() {
    vec3 center = (uOoosplatModelMatrix * vec4(getCenter(), 1.0)).xyz;
    bool deleted = loadOoosplatDeleted().r > 0.5;
    bool hit;
    if (uOoosplatSelectionOperation > 0.5) {
        hit = !deleted && !cropContains(center);
    } else {
        vec4 clip = uOoosplatViewProjection * vec4(center, 1.0);
        hit = !deleted
            && ooosplatFootprintHits(clip)
            && cropContains(center);
    }
    writeOoosplatScratch(vec4(hit ? 1.0 : 0.0));
}
`;

export const PROCESS_SELECTION_WGSL = /* wgsl */ `
uniform uOoosplatViewProjection: mat4x4f;
uniform uOoosplatModelMatrix: mat4x4f;
uniform uOoosplatSelectionRect: vec4f;
uniform uOoosplatSelectionCropKind: f32;
uniform uOoosplatSelectionCropCenter: vec3f;
uniform uOoosplatSelectionCropSize: vec3f;
uniform uOoosplatSelectionCropRadius: f32;
uniform uOoosplatSelectionCropInverted: f32;
uniform uOoosplatSelectionViewport: vec2f;
uniform uOoosplatSelectionOperation: f32;

fn cropContains(center: vec3f) -> bool {
    if (uniform.uOoosplatSelectionCropKind < 0.5) {
        return true;
    }
    let inside = select(
        all(abs(center - uniform.uOoosplatSelectionCropCenter) <= uniform.uOoosplatSelectionCropSize * 0.5),
        distance(center, uniform.uOoosplatSelectionCropCenter) <= uniform.uOoosplatSelectionCropRadius,
        uniform.uOoosplatSelectionCropKind < 1.5
    );
    return select(inside, !inside, uniform.uOoosplatSelectionCropInverted > 0.5);
}

fn ooosplatRotate(q: vec4f, v: vec3f) -> vec3f {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

fn ooosplatProjectedAxis(axis: vec3f, clip: vec4f) -> vec2f {
    let delta = uniform.uOoosplatViewProjection * uniform.uOoosplatModelMatrix * vec4f(axis, 0.0);
    return (delta.xy - clip.xy / clip.w * delta.w) / clip.w;
}

fn ooosplatEllipseDistance(p: vec2f, covariance: vec3f) -> f32 {
    let determinant = max(covariance.x * covariance.z - covariance.y * covariance.y, 1e-30);
    return (covariance.z * p.x * p.x - 2.0 * covariance.y * p.x * p.y + covariance.x * p.y * p.y) / determinant;
}

fn ooosplatEllipseHitsRect(center: vec2f, cov: vec3f, limit: f32) -> bool {
    let lo = uniform.uOoosplatSelectionRect.xy - center;
    let hi = uniform.uOoosplatSelectionRect.zw - center;
    if (all(lo <= vec2f(0.0)) && all(hi >= vec2f(0.0))) { return true; }
    let q0 = ooosplatEllipseDistance(vec2f(lo.x, clamp(cov.y * lo.x / cov.x, lo.y, hi.y)), cov);
    let q1 = ooosplatEllipseDistance(vec2f(hi.x, clamp(cov.y * hi.x / cov.x, lo.y, hi.y)), cov);
    let q2 = ooosplatEllipseDistance(vec2f(clamp(cov.y * lo.y / cov.z, lo.x, hi.x), lo.y), cov);
    let q3 = ooosplatEllipseDistance(vec2f(clamp(cov.y * hi.y / cov.z, lo.x, hi.x), hi.y), cov);
    return min(min(q0, q1), min(q2, q3)) <= limit;
}

fn ooosplatFootprintHits(clip: vec4f) -> bool {
    if (clip.w <= 0.0 || abs(clip.z) > clip.w) { return false; }
    let opacity = select(getColor().a, max(getColor().a, 0.72), loadOoosplatSelected().r > 0.5);
    if (opacity < 1.0 / 255.0) { return false; }
    let rotation = getRotation().yzwx;
    let scale = getScale();
    let x = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3f(scale.x, 0.0, 0.0)), clip);
    let y = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3f(0.0, scale.y, 0.0)), clip);
    let z = ooosplatProjectedAxis(ooosplatRotate(rotation, vec3f(0.0, 0.0, scale.z)), clip);
    let blur = 0.3 / max(uniform.uOoosplatSelectionViewport * uniform.uOoosplatSelectionViewport, vec2f(1.0));
    let cov = vec3f(dot(vec3f(x.x, y.x, z.x), vec3f(x.x, y.x, z.x)) + blur.x,
                    dot(vec3f(x.x, y.x, z.x), vec3f(x.y, y.y, z.y)),
                    dot(vec3f(x.y, y.y, z.y), vec3f(x.y, y.y, z.y)) + blur.y);
    let limit = -2.0 * log(exp(-4.0) + (1.0 - exp(-4.0)) / (255.0 * opacity));
    return ooosplatEllipseHitsRect(clip.xy / clip.w, cov, limit);
}

fn process() {
    let center = (uniform.uOoosplatModelMatrix * vec4f(getCenter(), 1.0)).xyz;
    let deleted = loadOoosplatDeleted().r > 0.5;
    var hit = false;
    if (uniform.uOoosplatSelectionOperation > 0.5) {
        hit = !deleted && !cropContains(center);
    } else {
        let clip = uniform.uOoosplatViewProjection * vec4f(center, 1.0);
        hit = !deleted
            && ooosplatFootprintHits(clip)
            && cropContains(center);
    }
    writeOoosplatScratch(vec4f(select(0.0, 1.0, hit)));
}
`;

function expandPackedMask(target: Uint8Array, packed: Uint8Array, splatCount: number) {
  target.fill(0);
  for (let index = 0; index < splatCount; index += 1) target[index] = maskBit(packed, index) ? 255 : 0;
}

function masksEqual(left: Uint8Array, right: Uint8Array) {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) if (left[index] !== right[index]) return false;
  return true;
}

export function packSelectionTextureData(data: Uint8Array, splatCount: number) {
  const packed = new Uint8Array(packedMaskLength(splatCount));
  for (let index = 0; index < splatCount; index += 1) if (data[index] > 127) setMaskBit(packed, index, true);
  return packed;
}

export function combineSelectionMasks(current: Uint8Array, hit: Uint8Array, mode: RectangleSelectionMode) {
  if (current.length !== hit.length) throw new Error(translate(getCurrentLocale(), "viewer.selectionMaskLength"));
  if (mode === "replace") return hit.slice();
  const next = current.slice();
  for (let index = 0; index < next.length; index += 1) {
    next[index] = mode === "add" ? next[index] | hit[index] : next[index] & ~hit[index];
  }
  return next;
}

export function combineDeletedMasks(current: Uint8Array, outsideCrop: Uint8Array) {
  if (current.length !== outsideCrop.length) throw new Error(translate(getCurrentLocale(), "viewer.deletedMaskLength"));
  const next = current.slice();
  for (let index = 0; index < next.length; index += 1) next[index] |= outsideCrop[index];
  return next;
}

export class GaussianSelectionController {
  private readonly processor: GSplatProcessor;
  private readonly selectedTexture: Texture;
  private readonly deletedTexture: Texture;
  private readonly scratchTexture: Texture;
  private readonly selectedPixels: Uint8Array;
  private readonly deletedPixels: Uint8Array;
  private readonly viewProjection = new Mat4();
  private selectedMask: Uint8Array;
  private deletedMask: Uint8Array;
  private destroyed = false;
  private queue: Promise<void> = Promise.resolve();
  private requestedSelectionRevision = 0;
  private appliedSelectionRevision = 0;

  constructor(
    private readonly device: GraphicsDevice,
    private readonly component: GSplatComponent,
    private readonly camera: CameraComponent,
    private readonly splatCount: number,
    private readonly requestRender: () => void,
  ) {
    const selected = component.getInstanceTexture("ooosplatSelected");
    const deleted = component.getInstanceTexture("ooosplatDeleted");
    const scratch = component.getInstanceTexture("ooosplatScratch");
    if (!selected || !deleted || !scratch) throw new Error(translate(getCurrentLocale(), "viewer.editTextures"));
    this.selectedTexture = selected;
    this.deletedTexture = deleted;
    this.scratchTexture = scratch;
    this.selectedPixels = new Uint8Array(selected.width * selected.height);
    this.deletedPixels = new Uint8Array(deleted.width * deleted.height);
    this.selectedMask = new Uint8Array(packedMaskLength(splatCount));
    this.deletedMask = new Uint8Array(packedMaskLength(splatCount));
    this.processor = new GSplatProcessor(
      device,
      { component },
      { component, streams: ["ooosplatScratch"] },
      { processGLSL: PROCESS_SELECTION_GLSL, processWGSL: PROCESS_SELECTION_WGSL },
    );
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    this.processor.destroy();
  }

  private enqueue<T>(operation: () => T | Promise<T>) {
    const result = this.queue.catch(() => undefined).then(operation);
    this.queue = result.then(() => undefined, () => undefined);
    return result;
  }

  private validateMask(mask: Uint8Array) {
    if (mask.length !== packedMaskLength(this.splatCount)) throw new Error(translate(getCurrentLocale(), "viewer.editMaskLength"));
  }

  private uploadMask(texture: Texture, pixels: Uint8Array, mask: Uint8Array) {
    expandPackedMask(pixels, mask, this.splatCount);
    const textureData = texture.lock() as Uint8Array;
    textureData.set(pixels);
    texture.unlock();
  }

  private refreshWorkBuffer() {
    if (this.destroyed) return;
    this.component.workBufferUpdate = WORKBUFFER_UPDATE_ONCE;
    this.requestRender();
  }

  applyMasks(deletedMask: Uint8Array, selectedMask: Uint8Array) {
    const deleted = deletedMask.slice();
    const selected = selectedMask.slice();
    return this.enqueue(() => {
      if (this.destroyed) return;
      this.validateMask(deleted);
      this.validateMask(selected);
      const deletedChanged = !masksEqual(this.deletedMask, deleted);
      const selectedChanged = !masksEqual(this.selectedMask, selected);
      if (!deletedChanged && !selectedChanged) return;
      if (deletedChanged) {
        this.deletedMask = deleted;
        this.uploadMask(this.deletedTexture, this.deletedPixels, deleted);
      }
      if (selectedChanged) {
        this.selectedMask = selected;
        this.uploadMask(this.selectedTexture, this.selectedPixels, selected);
      }
      this.refreshWorkBuffer();
    });
  }

  select(rectangle: SelectionRectangle, mode: RectangleSelectionMode, crop: GaussianCrop) {
    const revision = ++this.requestedSelectionRevision;
    const execute = async () => {
      if (this.destroyed) return this.selectedMask.slice();
      this.viewProjection.mul2(this.camera.projectionMatrix, this.camera.camera.viewMatrix);
      const cropKind = crop?.kind === "sphere" ? 1 : crop?.kind === "box" ? 2 : 0;
      const center = crop?.center ?? [0, 0, 0];
      const size = crop?.kind === "box" ? crop.size : [1, 1, 1];
      const radius = crop?.kind === "sphere" ? crop.radius : 1;
      this.processor.setParameter("uOoosplatViewProjection", this.viewProjection.data);
      this.processor.setParameter("uOoosplatModelMatrix", this.component.entity.getWorldTransform().data);
      this.processor.setParameter("uOoosplatSelectionRect", [rectangle.minX, rectangle.minY, rectangle.maxX, rectangle.maxY]);
      this.processor.setParameter("uOoosplatSelectionCropKind", cropKind);
      this.processor.setParameter("uOoosplatSelectionCropCenter", center);
      this.processor.setParameter("uOoosplatSelectionCropSize", size);
      this.processor.setParameter("uOoosplatSelectionCropRadius", radius);
      this.processor.setParameter("uOoosplatSelectionCropInverted", crop?.inverted ? 1 : 0);
      this.processor.setParameter("uOoosplatSelectionViewport", [this.device.width || 1, this.device.height || 1]);
      this.processor.setParameter("uOoosplatSelectionOperation", 0);
      this.processor.process();
      const pixels = await this.scratchTexture.read(0, 0, this.scratchTexture.width, this.scratchTexture.height, { immediate: true }) as Uint8Array;
      const hit = packSelectionTextureData(pixels, this.splatCount);
      const next = combineSelectionMasks(this.selectedMask, hit, mode);
      if (this.destroyed || revision < this.appliedSelectionRevision) return this.selectedMask.slice();
      this.selectedMask = next.slice();
      this.appliedSelectionRevision = revision;
      this.uploadMask(this.selectedTexture, this.selectedPixels, this.selectedMask);
      this.refreshWorkBuffer();
      return next;
    };
    return this.enqueue(execute);
  }

  freezeCrop(crop: Exclude<GaussianCrop, null>, deletedMask: Uint8Array) {
    const currentDeleted = deletedMask.slice();
    const revision = ++this.requestedSelectionRevision;
    const execute = async () => {
      if (this.destroyed) throw new Error(translate(getCurrentLocale(), "viewer.selectorDestroyed"));
      this.validateMask(currentDeleted);
      if (!masksEqual(this.deletedMask, currentDeleted)) {
        this.deletedMask = currentDeleted;
        this.uploadMask(this.deletedTexture, this.deletedPixels, this.deletedMask);
      }
      const cropKind = crop.kind === "sphere" ? 1 : 2;
      const size = crop.kind === "box" ? crop.size : [1, 1, 1];
      const radius = crop.kind === "sphere" ? crop.radius : 1;
      this.processor.setParameter("uOoosplatModelMatrix", this.component.entity.getWorldTransform().data);
      this.processor.setParameter("uOoosplatSelectionCropKind", cropKind);
      this.processor.setParameter("uOoosplatSelectionCropCenter", crop.center);
      this.processor.setParameter("uOoosplatSelectionCropSize", size);
      this.processor.setParameter("uOoosplatSelectionCropRadius", radius);
      this.processor.setParameter("uOoosplatSelectionCropInverted", crop.inverted ? 1 : 0);
      this.processor.setParameter("uOoosplatSelectionOperation", 1);
      this.processor.process();
      const pixels = await this.scratchTexture.read(0, 0, this.scratchTexture.width, this.scratchTexture.height, { immediate: true }) as Uint8Array;
      if (this.destroyed || revision < this.appliedSelectionRevision) throw new Error(translate(getCurrentLocale(), "viewer.cropExpired"));
      const outsideCrop = packSelectionTextureData(pixels, this.splatCount);
      const next = combineDeletedMasks(this.deletedMask, outsideCrop);
      this.deletedMask = next.slice();
      this.selectedMask = new Uint8Array(this.selectedMask.length);
      this.appliedSelectionRevision = revision;
      this.uploadMask(this.deletedTexture, this.deletedPixels, this.deletedMask);
      this.uploadMask(this.selectedTexture, this.selectedPixels, this.selectedMask);
      this.refreshWorkBuffer();
      return next;
    };
    return this.enqueue(execute);
  }

  invertSelection() {
    const revision = ++this.requestedSelectionRevision;
    return this.enqueue(() => {
      if (this.destroyed) throw new Error(translate(getCurrentLocale(), "viewer.selectorDestroyed"));
      for (let index = 0; index < this.selectedMask.length; index += 1) {
        this.selectedMask[index] = ~this.selectedMask[index] & ~this.deletedMask[index];
      }
      const remainder = this.splatCount % 8;
      if (remainder) this.selectedMask[this.selectedMask.length - 1] &= (1 << remainder) - 1;
      this.appliedSelectionRevision = revision;
      this.uploadMask(this.selectedTexture, this.selectedPixels, this.selectedMask);
      this.refreshWorkBuffer();
      return this.selectedMask.slice();
    });
  }
}
