import {
  Color, Entity, Gizmo, RotateGizmo, TransformGizmo, TranslateGizmo,
  type Application, type CameraComponent, type Layer, type RenderComponent,
} from "playcanvas";
import type { GaussianTransform } from "../../types/pipeline";
import type { GizmoMode } from "./GizmoPreferences";
import { modelWorldCenter, nearestEuler, rotateAroundModelCenter, translatedFromProxy, type ModelPivot } from "./ModelTransformMath";

interface GizmoCallbacks {
  /** Null means this scene no longer owns the editor session. */
  current: () => GaussianTransform | null;
  beforeStart: () => void;
  begin: () => void;
  change: (transform: GaussianTransform) => void;
  finish: (cancel: boolean) => void;
  dragging: (active: boolean) => void;
}

/** One native gizmo at a time, attached to a center proxy, never the PLY node.
 * Only public PlayCanvas APIs are used; the model's persisted transform remains
 * position/Euler/uniform scale. Scene resources cannot leak across sessions.
 */
export class ModelTransformGizmo {
  private readonly proxy = new Entity("OOOSplat Model Pivot");
  private readonly layer: Layer;
  private gizmo: TranslateGizmo | RotateGizmo | null = null;
  private mode: GizmoMode = "off";
  private active = false;
  private destroyed = false;
  private gesture: GaussianTransform | null = null;
  private pointerId: number | null = null;
  private handles: Array<{ off: () => void }> = [];

  constructor(
    private readonly app: Application,
    private readonly camera: CameraComponent,
    readonly pivot: ModelPivot,
    private readonly callbacks: GizmoCallbacks,
  ) {
    this.layer = Gizmo.createLayer(app, "OOOSplat Transform Axes");
    app.root.addChild(this.proxy);
    const canvas = app.graphicsDevice.canvas;
    canvas.addEventListener("pointerdown", this.beforePointerDown, true);
    canvas.addEventListener("pointercancel", this.pointerInterrupted);
    canvas.addEventListener("lostpointercapture", this.pointerInterrupted);
    window.addEventListener("blur", this.pointerInterrupted);
    window.addEventListener("keydown", this.keyDown, true);
  }

  private beforePointerDown = (event: PointerEvent) => {
    if (!this.active || this.destroyed || event.button !== 0 || this.gesture) return;
    // Blur before the native gizmo snapshots its proxy, not in transform:start.
    this.callbacks.beforeStart();
    this.sync();
    this.pointerId = event.pointerId;
  };

  private pointerInterrupted = (event: Event) => {
    if ("pointerId" in event && this.pointerId !== null && event.pointerId !== this.pointerId) return;
    this.finish();
  };
  private keyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || !this.gesture) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.finish(true);
  };

  configure(mode: GizmoMode, enabled: boolean) {
    if (this.destroyed) return;
    const active = enabled && mode !== "off";
    if (mode !== this.mode) {
      this.finish();
      this.releaseGizmo();
      this.mode = mode;
    }
    if (!active) {
      this.active = false;
      this.finish();
      this.gizmo?.detach();
    } else {
      this.active = true;
      if (!this.gizmo) this.createGizmo();
      this.sync();
      if (this.gizmo!.nodes.length === 0) this.gizmo!.attach([this.proxy]);
    }
    this.app.renderNextFrame = true;
  }

  sync() {
    if (this.destroyed || this.gesture) return;
    const transform = this.callbacks.current();
    if (!transform) return;
    this.proxy.setPosition(modelWorldCenter(transform, this.pivot));
    this.proxy.setEulerAngles(...transform.rotation);
    this.gizmo?.update();
    this.app.renderNextFrame = true;
  }

  private createGizmo() {
    const gizmo = this.mode === "rotate" ? new RotateGizmo(this.camera, this.layer) : new TranslateGizmo(this.camera, this.layer);
    this.gizmo = gizmo;
    gizmo.coordSpace = "world";
    gizmo.size = 0.9;
    // Hide only the scene-spanning direction guides. Shape/hover colors also
    // drive RotateGizmo's two short angle guides and must remain untouched.
    gizmo.setTheme({
      guideBase: { x: new Color(0, 0, 0, 0), y: new Color(0, 0, 0, 0), z: new Color(0, 0, 0, 0) },
      guideOcclusion: 1,
    });
    gizmo.mouseButtons[1] = false;
    gizmo.mouseButtons[2] = false;
    // Remove plane/free-move/trackball handles: blank space remains camera orbit.
    const hiddenShapes: typeof gizmo.intersectShapes = [];
    for (const shape of gizmo.intersectShapes) {
      const keep = this.mode === "translate" ? /^arrow:[xyz]$/.test(shape.entity.name) : /^disk:[xyz]$/.test(shape.entity.name);
      if (!keep) { shape.disabled = true; shape.visible = false; shape.entity.enabled = false; hiddenShapes.push(shape); }
    }
    gizmo.intersectShapes = gizmo.intersectShapes.filter(shape => !shape.disabled);
    this.handles = [
      gizmo.on(TransformGizmo.EVENT_TRANSFORMSTART, () => {
        const current = this.callbacks.current();
        if (!this.active || this.destroyed || !current) return;
        this.gesture = { position: [...current.position], rotation: [...current.rotation], scale: current.scale };
        this.callbacks.begin();
        this.callbacks.dragging(true);
      }),
      gizmo.on(TransformGizmo.EVENT_TRANSFORMMOVE, () => {
        const current = this.callbacks.current();
        if (!this.gesture || !current || this.destroyed) return;
        const next = this.mode === "translate"
          ? translatedFromProxy(this.gesture, this.pivot, this.proxy.getPosition())
          : rotateAroundModelCenter(this.gesture, this.pivot, nearestEuler(this.proxy.getRotation(), current.rotation));
        if ([...next.position, ...next.rotation, next.scale].every(Number.isFinite)) this.callbacks.change(next);
        this.app.renderNextFrame = true;
      }),
      gizmo.on(TransformGizmo.EVENT_TRANSFORMEND, () => this.finish()),
      gizmo.on(Gizmo.EVENT_RENDERUPDATE, () => { this.app.renderNextFrame = true; }),
      // Native camera-facing and drag updates can re-enable their default handles.
      // Keep excluded handles hidden after those updates, using only public APIs.
      this.app.on("prerender", () => {
        hiddenShapes.forEach(shape => { shape.entity.enabled = false; shape.visible = false; });
      }),
    ];
  }

  /** Forced ends also reset the native gesture via detach, so late moves are inert. */
  finish(cancel = false) {
    if (!this.gesture) return;
    this.gesture = null;
    this.callbacks.finish(cancel);
    this.gizmo?.detach();
    this.sync();
    if (this.active && !this.destroyed) this.gizmo?.attach([this.proxy]);
    const canvas = this.app.graphicsDevice.canvas;
    if (this.pointerId !== null && canvas.hasPointerCapture(this.pointerId)) canvas.releasePointerCapture(this.pointerId);
    this.pointerId = null;
    this.callbacks.dragging(false);
    this.app.renderNextFrame = true;
  }

  private releaseGizmo() {
    this.handles.forEach(handle => handle.off());
    this.handles = [];
    if (!this.gizmo) return;
    // Shape.destroy releases entities/mesh instances but not their shared materials.
    const materials = new Set(this.gizmo.intersectShapes.flatMap(shape => shape.meshInstances.map(instance => instance.material)));
    // Include hidden native handles too, through the public entity hierarchy.
    for (const render of this.gizmo.root.findComponents("render") as RenderComponent[]) for (const instance of render.meshInstances) materials.add(instance.material);
    this.gizmo.destroy();
    materials.forEach(material => material.destroy());
    this.camera.layers = this.camera.layers.filter(id => id !== this.layer.id);
    this.gizmo = null;
  }

  destroy(cancel = false) {
    if (this.destroyed) return;
    this.active = false;
    this.finish(cancel);
    this.destroyed = true;
    const canvas = this.app.graphicsDevice.canvas;
    canvas.removeEventListener("pointerdown", this.beforePointerDown, true);
    canvas.removeEventListener("pointercancel", this.pointerInterrupted);
    canvas.removeEventListener("lostpointercapture", this.pointerInterrupted);
    window.removeEventListener("blur", this.pointerInterrupted);
    window.removeEventListener("keydown", this.keyDown, true);
    this.releaseGizmo();
    this.proxy.destroy();
    this.app.scene.layers.remove(this.layer);
  }
}
