import { Quat, Vec3 } from "playcanvas";
import type { GaussianTransform } from "../../types/pipeline";
import { plyDirectionToEngine } from "./CoordinateSystem";

export type ModelPivot = [number, number, number];

/** The cached PLY center expressed in the transform node's local coordinates. */
export function modelPivotFromPly(center: Vec3): ModelPivot {
  const pivot = plyDirectionToEngine(center);
  return [pivot.x, pivot.y, pivot.z];
}

export function modelWorldCenter(transform: GaussianTransform, pivot: ModelPivot): Vec3 {
  const offset = new Quat().setFromEulerAngles(...transform.rotation)
    .transformVector(new Vec3(...pivot).mulScalar(transform.scale));
  return offset.add(new Vec3(...transform.position));
}

/** Both panel edits and scene rotation preserve this same world-space center. */
export function rotateAroundModelCenter(
  transform: GaussianTransform, pivot: ModelPivot, rotation: GaussianTransform["rotation"],
): GaussianTransform {
  const center = modelWorldCenter(transform, pivot);
  const offset = new Quat().setFromEulerAngles(...rotation)
    .transformVector(new Vec3(...pivot).mulScalar(transform.scale));
  const position = center.sub(offset);
  return { position: [position.x, position.y, position.z], rotation: [...rotation], scale: transform.scale };
}

/** Choose the equivalent Euler representation closest to the current panel values. */
export function nearestEuler(rotation: Quat, previous: GaussianTransform["rotation"]): GaussianTransform["rotation"] {
  const euler = rotation.getEulerAngles();
  const nearest = (angle: number, reference: number) => angle + 360 * Math.round((reference - angle) / 360);
  const candidates = [[euler.x, euler.y, euler.z], [euler.x + 180, 180 - euler.y, euler.z + 180]]
    .map(angles => angles.map((angle, index) => nearest(angle, previous[index])) as GaussianTransform["rotation"]);
  return candidates.sort((a, b) => a.reduce((sum, value, i) => sum + (value - previous[i]) ** 2, 0)
    - b.reduce((sum, value, i) => sum + (value - previous[i]) ** 2, 0))[0];
}

export function translatedFromProxy(
  start: GaussianTransform, pivot: ModelPivot, proxyPosition: Vec3,
): GaussianTransform {
  const delta = proxyPosition.clone().sub(modelWorldCenter(start, pivot));
  return { ...start, position: start.position.map((value, index) => value + [delta.x, delta.y, delta.z][index]) as ModelPivot };
}
