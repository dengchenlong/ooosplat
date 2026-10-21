import { Mat4, Quat, Vec3 } from "playcanvas";
import { describe, expect, it } from "vitest";
import { trainingReferenceInWorld } from "./TrainingReferenceView";
import { PLY_TO_ENGINE_ROTATION } from "./CoordinateSystem";
import type { TrainingReferenceView } from "../../types/pipeline";

const camera: TrainingReferenceView = { imageName: "frame.png", position: [1, 2, 3], forward: [0, 0, 1], up: [0, -1, 0],
  fov: 80, projectionOffset: [0.01, -0.02], projectionScaleX: 1.05 };

describe("training camera coordinate alignment", () => {
  it("applies the same 180-degree PLY mapping and edit transform as the Gaussian node", () => {
    const transform = { position: [5, 6, 7] as [number, number, number], rotation: [22, -67, 13] as [number, number, number], scale: 2.5 };
    const view = trainingReferenceInWorld(camera, transform);
    const ply = new Mat4().setTRS(Vec3.ZERO, new Quat().setFromEulerAngles(...PLY_TO_ENGINE_ROTATION), Vec3.ONE);
    const edits = new Mat4().setTRS(new Vec3(...transform.position), new Quat().setFromEulerAngles(...transform.rotation), new Vec3(2.5, 2.5, 2.5));
    const world = new Mat4().mul2(edits, ply);
    expect(view.position.distance(world.transformPoint(new Vec3(...camera.position)))).toBeLessThan(1e-5);
    expect(view.forward.dot(world.transformVector(new Vec3(...camera.forward)).normalize())).toBeCloseTo(1);
    expect(view.up.dot(view.forward)).toBeCloseTo(0);
    expect(view.projectionOffset).toEqual(camera.projectionOffset);
    expect(view.fov).toBe(80);
    expect(view.projectionScaleX).toBe(1.05);
    expect(camera.position).toEqual([1, 2, 3]);
  });
});
