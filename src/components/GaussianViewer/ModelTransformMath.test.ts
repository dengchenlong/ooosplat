import { describe, expect, it } from "vitest";
import { Quat, Vec3 } from "playcanvas";
import type { GaussianTransform } from "../../types/pipeline";
import { modelPivotFromPly, modelWorldCenter, nearestEuler, rotateAroundModelCenter, translatedFromProxy } from "./ModelTransformMath";

describe("model center transforms", () => {
  it("maps the cached PLY pivot and keeps it fixed for panel and world-axis rotations at different scales", () => {
    const pivot = modelPivotFromPly(new Vec3(2, -3, 4));
    expect(pivot[0]).toBeCloseTo(-2); expect(pivot[1]).toBeCloseTo(3); expect(pivot[2]).toBe(4);
    for (const scale of [0.001, 1, 1500, 10000]) {
      const original: GaussianTransform = { position: [10, 20, -30], rotation: [18, 36, -42], scale };
      const center = modelWorldCenter(original, pivot);
      const panel = rotateAroundModelCenter(original, pivot, [18, 128, -42]);
      expect(modelWorldCenter(panel, pivot).distance(center)).toBeLessThan(1e-8);
      const worldRotation = new Quat().setFromAxisAngle(new Vec3(1, 0, 0), 95)
        .mul(new Quat().setFromEulerAngles(...original.rotation));
      const euler = nearestEuler(worldRotation, original.rotation);
      const axes = rotateAroundModelCenter(original, pivot, euler);
      expect(modelWorldCenter(axes, pivot).distance(center)).toBeLessThan(1e-8);
      expect(Math.abs(new Quat().setFromEulerAngles(...euler).dot(worldRotation))).toBeCloseTo(1, 6);
      expect(axes.scale).toBe(scale);
      expect(original.position).toEqual([10, 20, -30]);
    }
  });

  it("moves the proxy and model by the same world delta without changing rotation or scale", () => {
    const original: GaussianTransform = { position: [4, 9, 12], rotation: [30, 60, 90], scale: 2 };
    const pivot: [number, number, number] = [-10, 20, 30];
    const center = modelWorldCenter(original, pivot);
    const result = translatedFromProxy(original, pivot, center.clone().add(new Vec3(3, -4, 5)));
    expect(result.position).toEqual([7, 5, 17]);
    expect(result.rotation).toEqual(original.rotation); expect(result.scale).toBe(2);
  });

  it("keeps Euler values continuous across the 180-degree representation boundary", () => {
    const result = nearestEuler(new Quat().setFromEulerAngles(181, 10, -359), [179, 10, -359]);
    expect(result[0]).toBeCloseTo(181); expect(result[1]).toBeCloseTo(10); expect(result[2]).toBeCloseTo(-359);
    for (const angles of [[90, 0, 0], [0, 90, 0], [60, 90, 30], [160, 95, -170], [-270, 450, 810]] as [number, number, number][]) {
      const quaternion = new Quat().setFromEulerAngles(...angles);
      const equivalent = nearestEuler(quaternion, angles);
      expect(Math.abs(new Quat().setFromEulerAngles(...equivalent).dot(quaternion))).toBeCloseTo(1, 6);
    }
  });
});
