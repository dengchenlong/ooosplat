import { Color, LAYERID_WORLD, Vec3, type Application } from "playcanvas";
import type { ViewerControls, ViewerOrbitGuideState } from "./ViewerControls";

const RING_SEGMENTS = 96;
const AXIS_DASHES = 18;

export interface OrbitAxisGuideGeometry {
  positions: Vec3[];
  colors: Color[];
}

function orbitPoint(target: Vec3, radius: number, height: number, angle: number) {
  return new Vec3(
    target.x + Math.sin(angle) * radius,
    height,
    target.z + Math.cos(angle) * radius,
  );
}

export function buildOrbitAxisGuide(
  state: ViewerOrbitGuideState,
  modelSpan: number,
): OrbitAxisGuideGeometry {
  const target = new Vec3(...state.target);
  const radius = Math.max(state.radius, modelSpan * 0.08, 0.0001);
  const referenceSpan = Math.max(modelSpan, radius * 0.7, 0.0001);
  const axisBottom = Math.min(target.y - referenceSpan * 0.55, state.height - referenceSpan * 0.18);
  const axisTop = Math.max(target.y + referenceSpan * 0.65, state.height + referenceSpan * 0.22);
  const positions: Vec3[] = [];
  const colors: Color[] = [];
  const axisColor = new Color(0.58, 0.75, 1, 0.82);
  const ringColor = new Color(0.117647, 0.360784, 1, 0.34);
  const ringAccent = new Color(0.38, 0.61, 1, 0.58);
  const arrowColor = new Color(0.76, 0.86, 1, 0.95);
  const centerColor = new Color(0.9, 0.95, 1, 0.95);

  const addLine = (start: Vec3, end: Vec3, color: Color) => {
    positions.push(start, end);
    colors.push(color, color);
  };

  const axisLength = axisTop - axisBottom;
  for (let index = 0; index < AXIS_DASHES; index += 1) {
    const start = axisBottom + axisLength * index / AXIS_DASHES;
    const end = axisBottom + axisLength * (index + 0.62) / AXIS_DASHES;
    addLine(new Vec3(target.x, start, target.z), new Vec3(target.x, end, target.z), axisColor);
  }

  const axisArrowSize = Math.max(referenceSpan * 0.055, radius * 0.025);
  const axisTip = new Vec3(target.x, axisTop, target.z);
  addLine(axisTip, new Vec3(target.x - axisArrowSize, axisTop - axisArrowSize * 1.7, target.z), arrowColor);
  addLine(axisTip, new Vec3(target.x + axisArrowSize, axisTop - axisArrowSize * 1.7, target.z), arrowColor);
  addLine(axisTip, new Vec3(target.x, axisTop - axisArrowSize * 1.7, target.z - axisArrowSize), arrowColor);
  addLine(axisTip, new Vec3(target.x, axisTop - axisArrowSize * 1.7, target.z + axisArrowSize), arrowColor);

  for (let index = 0; index < RING_SEGMENTS; index += 1) {
    const startAngle = index / RING_SEGMENTS * Math.PI * 2;
    const endAngle = (index + 1) / RING_SEGMENTS * Math.PI * 2;
    addLine(
      orbitPoint(target, radius, state.height, startAngle),
      orbitPoint(target, radius, state.height, endAngle),
      index % 12 < 3 ? ringAccent : ringColor,
    );
  }

  const directionAngle = (state.angleDegrees + 70) * Math.PI / 180;
  const arrowTip = orbitPoint(target, radius, state.height, directionAngle);
  const tangent = new Vec3(Math.cos(directionAngle), 0, -Math.sin(directionAngle));
  const radial = new Vec3(Math.sin(directionAngle), 0, Math.cos(directionAngle));
  const arrowLength = Math.max(radius * 0.09, referenceSpan * 0.045);
  const arrowTail = arrowTip.clone().sub(tangent.clone().mulScalar(arrowLength));
  addLine(arrowTail, arrowTip, arrowColor);
  addLine(
    arrowTip,
    arrowTip.clone().sub(tangent.clone().mulScalar(arrowLength * 0.42)).add(radial.clone().mulScalar(arrowLength * 0.25)),
    arrowColor,
  );
  addLine(
    arrowTip,
    arrowTip.clone().sub(tangent.clone().mulScalar(arrowLength * 0.42)).sub(radial.clone().mulScalar(arrowLength * 0.25)),
    arrowColor,
  );

  const centerSize = Math.max(referenceSpan * 0.025, radius * 0.012);
  addLine(
    new Vec3(target.x - centerSize, target.y, target.z),
    new Vec3(target.x + centerSize, target.y, target.z),
    centerColor,
  );
  addLine(
    new Vec3(target.x, target.y, target.z - centerSize),
    new Vec3(target.x, target.y, target.z + centerSize),
    centerColor,
  );

  return { positions, colors };
}

export class OrbitAxisGuide {
  private destroyed = false;
  private visible = false;
  private modelSpan: number;
  private geometryKey = "";
  private geometry: OrbitAxisGuideGeometry | null = null;
  private readonly stopDrawing: () => void;

  constructor(app: Application, controls: ViewerControls, modelSpan: number) {
    this.modelSpan = Math.max(modelSpan, 0.0001);
    const worldLayer = app.scene.layers.getLayerById(LAYERID_WORLD) ?? undefined;
    const draw = () => {
      if (!this.visible) return;
      const state = controls.orbitGuideState();
      const key = [...state.target, state.radius, state.height, state.angleDegrees, this.modelSpan].join(",");
      if (key !== this.geometryKey || !this.geometry) {
        this.geometry = buildOrbitAxisGuide(state, this.modelSpan);
        this.geometryKey = key;
      }
      const geometry = this.geometry;
      app.drawLines(geometry.positions, geometry.colors, false, worldLayer);
    };
    const updateHandle = app.on("prerender", draw);
    this.stopDrawing = () => updateHandle.off();
  }

  setVisible(visible: boolean) {
    this.visible = visible;
  }

  setModelSpan(modelSpan: number) {
    if (Number.isFinite(modelSpan)) this.modelSpan = Math.max(modelSpan, 0.0001);
  }

  get isVisible() {
    return this.visible;
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    this.stopDrawing();
  }
}
