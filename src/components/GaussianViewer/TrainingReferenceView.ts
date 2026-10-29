import { Quat, Vec3 } from "playcanvas";
import type { GaussianTransform, TrainingReferenceView } from "../../types/pipeline";
import { plyDirectionToEngine } from "./CoordinateSystem";
import type { ReferenceView } from "./ViewerControls";

/** Poses and Gaussians share the same PLY-to-engine mapping and edit transform. */
export function trainingReferenceInWorld(view: TrainingReferenceView, transform: GaussianTransform): ReferenceView {
  const rotation = new Quat().setFromEulerAngles(...transform.rotation);
  const direction = (values: [number, number, number]) => rotation.transformVector(plyDirectionToEngine(new Vec3(...values))).normalize();
  const position = rotation.transformVector(plyDirectionToEngine(new Vec3(...view.position)).mulScalar(transform.scale))
    .add(new Vec3(...transform.position));
  return { position, forward: direction(view.forward), up: direction(view.up), fov: view.fov,
    projectionOffset: [...view.projectionOffset], projectionScaleX: view.projectionScaleX };
}
