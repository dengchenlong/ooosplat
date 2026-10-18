// @vitest-environment jsdom

import { BoundingBox, GraphNode, Mat4, PROJECTION_ORTHOGRAPHIC, PROJECTION_PERSPECTIVE, Vec3, type Entity } from "playcanvas";
import { describe, expect, it, vi } from "vitest";
import { ViewerControls } from "./ViewerControls";
import { buildGroundGrid } from "./GroundGrid";

function pointerEvent(type: string, x: number, y: number, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button, buttons: type === "pointerup" ? 0 : 1, clientX: x, clientY: y });
  Object.defineProperty(event, "pointerId", { value: 7 });
  return event;
}

function setup() {
  const canvas = document.createElement("canvas");
  const captures = new Set<number>();
  Object.defineProperties(canvas, {
    clientWidth: { value: 1200 },
    clientHeight: { value: 800 },
    setPointerCapture: { value: (id: number) => captures.add(id) },
    releasePointerCapture: { value: (id: number) => captures.delete(id) },
    hasPointerCapture: { value: (id: number) => captures.has(id) },
  });
  const camera = { fov: 52, aspectRatio: 1.5, nearClip: 0.01, farClip: 10_000,
    projection: PROJECTION_PERSPECTIVE, orthoHeight: 5 };
  const entity = new GraphNode();
  Object.defineProperty(entity, "camera", { value: camera });
  const setPosition = vi.spyOn(entity, "setPosition");
  const lookAt = vi.spyOn(entity, "lookAt");
  const onOrthographicViewChange = vi.fn();
  const controls = new ViewerControls(canvas, entity as Entity, onOrthographicViewChange);
  return { canvas, camera, controls, entity, setPosition, lookAt, onOrthographicViewChange };
}

describe("ViewerControls", () => {
  it("does not move the camera for a click or sub-threshold pointer motion", () => {
    const { canvas, controls, setPosition } = setup();
    const initialUpdates = setPosition.mock.calls.length;

    canvas.dispatchEvent(pointerEvent("pointerdown", 40, 40));
    canvas.dispatchEvent(pointerEvent("pointermove", 41, 41));
    canvas.dispatchEvent(pointerEvent("pointerup", 41, 41));

    expect(setPosition).toHaveBeenCalledTimes(initialUpdates);
    controls.destroy();
  });

  it("cancels an active camera gesture when another control takes ownership", () => {
    const { canvas, controls, setPosition } = setup();
    const initialUpdates = setPosition.mock.calls.length;

    canvas.dispatchEvent(pointerEvent("pointerdown", 20, 20));
    controls.cancelGesture();
    canvas.dispatchEvent(pointerEvent("pointermove", 80, 80));

    expect(setPosition).toHaveBeenCalledTimes(initialUpdates);
    controls.destroy();
  });

  it("limits a single wheel event and can be destroyed repeatedly", () => {
    const { canvas, controls } = setup();
    const before = controls.cameraDistance;

    canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: -1_000_000 }));

    expect(controls.cameraDistance).toBeLessThan(before);
    expect(controls.cameraDistance).toBeGreaterThanOrEqual(before * Math.exp(-0.24));
    expect(() => { controls.destroy(); controls.destroy(); }).not.toThrow();
  });

  it("starts above the model and resets to an upright viewing direction", () => {
    const { controls, setPosition } = setup();
    const initialPosition = setPosition.mock.calls.at(-1)?.[0] as Vec3;

    expect(initialPosition.y).toBeGreaterThan(0);
    controls.fit(new BoundingBox(new Vec3(), new Vec3(1, 1, 1)), { resetDirection: true });

    const resetPosition = setPosition.mock.calls.at(-1)?.[0] as Vec3;
    expect(controls.cameraPitch).toBe(22);
    expect(resetPosition.y).toBeGreaterThan(0);
    controls.destroy();
  });

  it("frames tightly and shifts the model away from the transform panel", () => {
    const { controls, lookAt } = setup();
    const bounds = new BoundingBox(new Vec3(), new Vec3(1, 1, 1));
    const oldBoundingSphereDistance = bounds.halfExtents.length() / Math.sin(26 * Math.PI / 180) * 1.18;

    controls.fit(bounds, { resetDirection: true, occupancy: 0.85, rightInsetPx: 286 });

    const viewTarget = lookAt.mock.calls.at(-1)?.[0] as Vec3;
    expect(controls.cameraDistance).toBeLessThan(oldBoundingSphereDistance);
    expect(viewTarget.x).toBeGreaterThan(bounds.center.x);
    controls.destroy();
  });

  it("extends the perspective far plane to include the ground grid", () => {
    const { camera, controls } = setup();
    controls.setSceneBounds(new BoundingBox(new Vec3(0, 1, 0), new Vec3(50, 1, 50)));
    controls.fit(new BoundingBox(new Vec3(), new Vec3(1, 1, 1)), { occupancy: 0.85 });

    expect(camera.nearClip).toBeLessThanOrEqual(0.001);
    expect(camera.farClip).toBeGreaterThan(50);
    controls.destroy();
  });

  it("keeps the original grid inside the orthographic depth range without changing screen framing", () => {
    const { camera, controls, entity } = setup();
    const model = new BoundingBox(new Vec3(), new Vec3(1.5, 1, 1));
    const grid = buildGroundGrid(model);
    controls.alignOrthographic("front", model, 0.85, false);
    controls.restore({ ...controls.snapshot(), orthographicView: null, yaw: 45, pitch: 22 });
    const saved = controls.snapshot();
    const points = [...grid.positions, new Vec3(0.5, 1, -0.5)];
    const project = (point: Vec3) => {
      const view = new Mat4().invert(entity.getWorldTransform());
      const local = view.transformPoint(point);
      return [local.x / (camera.orthoHeight * camera.aspectRatio), local.y / camera.orthoHeight];
    };
    const before = points.map(project);
    expect(Math.min(...grid.positions.map(point => point.clone().sub(entity.getPosition()).dot(entity.forward)))).toBeLessThan(0);

    const scene = model.clone();
    scene.add(grid.bounds);
    controls.setSceneBounds(scene);

    for (const [index, point] of points.entries()) {
      const depth = point.clone().sub(entity.getPosition()).dot(entity.forward);
      expect(depth).toBeGreaterThan(camera.nearClip);
      expect(depth).toBeLessThan(camera.farClip);
      const after = project(point);
      expect(after[0]).toBeCloseTo(before[index][0], 5);
      expect(after[1]).toBeCloseTo(before[index][1], 5);
    }
    expect(controls.snapshot()).toEqual(saved);
    controls.restore(saved);
    for (const point of grid.positions) {
      expect(point.clone().sub(entity.getPosition()).dot(entity.forward)).toBeGreaterThan(camera.nearClip);
    }
    controls.destroy();
  });

  it("keeps the full grid and translated model in range while orbiting, panning, zooming and realigning", () => {
    const { camera, controls, entity, canvas } = setup();
    const model = new BoundingBox(new Vec3(2, -1, 3), new Vec3(1.5, 1, 1));
    const grid = buildGroundGrid(model);
    const scene = model.clone();
    scene.add(grid.bounds);
    controls.setSceneBounds(scene);
    const points = [...grid.positions];
    for (const x of [-1, 1]) for (const y of [-1, 1]) for (const z of [-1, 1]) {
      points.push(new Vec3(model.center.x + x * model.halfExtents.x, model.center.y + y * model.halfExtents.y, model.center.z + z * model.halfExtents.z));
    }
    const checkDepths = () => {
      for (const point of points) {
        const depth = point.clone().sub(entity.getPosition()).dot(entity.forward);
        expect(depth).toBeGreaterThan(camera.nearClip);
        expect(depth).toBeLessThan(camera.farClip);
      }
    };
    for (const view of ["front", "side", "top"] as const) {
      controls.alignOrthographic(view, model, 0.85, false);
      checkDepths();
      canvas.dispatchEvent(pointerEvent("pointerdown", 40, 40));
      canvas.dispatchEvent(pointerEvent("pointermove", 60, 30));
      canvas.dispatchEvent(pointerEvent("pointerup", 60, 30));
      checkDepths();
      for (const pitch of [-70, -5, 0.1, 22, 70, 89.9]) {
        for (const yaw of [0, 45, 90, 180, 270]) {
          controls.restore({ ...controls.snapshot(), pitch, yaw });
          checkDepths();
        }
      }
      canvas.dispatchEvent(pointerEvent("pointerdown", 40, 40, 2));
      canvas.dispatchEvent(pointerEvent("pointermove", 100, 80, 2));
      canvas.dispatchEvent(pointerEvent("pointerup", 100, 80, 2));
      checkDepths();
      canvas.dispatchEvent(new WheelEvent("wheel", { cancelable: true, deltaY: 240 }));
      checkDepths();
    }
    const transformed = new BoundingBox(new Vec3(-10, 5, -8), new Vec3(4, 3, 2));
    const updated = transformed.clone();
    updated.add(grid.bounds);
    controls.setSceneBounds(updated);
    checkDepths();
    controls.destroy();
  });

  it("snapshots, orbits, and restores the exact interactive camera state", () => {
    const { controls } = setup();
    const initial = controls.snapshot();

    controls.orbitBy(15);
    expect(controls.snapshot().yaw).toBe(initial.yaw + 15);

    controls.restore(initial);
    expect(controls.snapshot()).toEqual(initial);
    controls.destroy();
  });

  it("reports the current Y-axis orbit center, radius, height, and direction", () => {
    const { controls } = setup();
    controls.fit(new BoundingBox(new Vec3(3, 4, 5), new Vec3(2, 1, 2)), { occupancy: 0.85 });
    const initial = controls.orbitGuideState();

    expect(initial.target).toEqual([3, 4, 5]);
    expect(initial.radius).toBeGreaterThan(0);
    expect(initial.height).toBeGreaterThan(initial.target[1]);

    controls.orbitBy(15);
    const rotated = controls.orbitGuideState();
    expect(rotated.radius).toBeCloseTo(initial.radius);
    expect(rotated.height).toBeCloseTo(initial.height);
    expect(rotated.angleDegrees).toBeCloseTo(initial.angleDegrees + 15);
    controls.destroy();
  });

  it("restores the orthographic view indicator with the camera", () => {
    const { controls, onOrthographicViewChange } = setup();
    controls.alignOrthographic("top", new BoundingBox(new Vec3(1, 2, 3), new Vec3(2, 3, 4)), 0.85, false);
    const saved = controls.snapshot();
    controls.usePerspective();
    controls.restore(saved);
    expect(controls.snapshot()).toEqual(saved);
    expect(onOrthographicViewChange).toHaveBeenLastCalledWith("top");
    controls.destroy();
  });

  it("animates between orthographic views and pauses the transition while the model gizmo owns input", () => {
    const frames: FrameRequestCallback[] = [];
    const request = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
    const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
    const clock = vi.spyOn(performance, "now").mockReturnValue(1000);
    const { camera, controls, setPosition } = setup();
    const before = setPosition.mock.calls.length;

    controls.alignOrthographic("side", new BoundingBox(new Vec3(2, 3, 4), new Vec3(5, 6, 7)));
    expect(camera.projection).toBe(PROJECTION_ORTHOGRAPHIC);
    expect(frames).toHaveLength(1);
    frames.shift()?.(1160);
    expect(setPosition.mock.calls.length).toBeGreaterThan(before);
    expect(frames).toHaveLength(1);
    controls.enabled = false;
    const pausedUpdates = setPosition.mock.calls.length;
    frames.shift()?.(2160);
    controls.setSceneBounds(new BoundingBox(new Vec3(2, 3, 4), new Vec3(6, 7, 8)));
    frames.shift()?.(3160);
    expect(setPosition).toHaveBeenCalledTimes(pausedUpdates);
    expect(frames).toHaveLength(1);
    controls.enabled = true;
    frames.shift()?.(3320);
    expect(setPosition.mock.calls.length).toBeGreaterThan(pausedUpdates);
    expect(frames).toHaveLength(0);

    controls.destroy();
    request.mockRestore();
    cancel.mockRestore();
    clock.mockRestore();
  });

  it("keeps grid vertices in front of the near plane on every orthographic transition frame", () => {
    const frames: FrameRequestCallback[] = [];
    const request = vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => {
      frames.push(callback);
      return frames.length;
    });
    const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
    const clock = vi.spyOn(performance, "now").mockReturnValue(0);
    const { camera, controls, entity } = setup();
    const model = new BoundingBox(new Vec3(), new Vec3(1.5, 1, 1));
    const grid = buildGroundGrid(model);
    const scene = model.clone();
    scene.add(grid.bounds);
    controls.setSceneBounds(scene);
    try {
      for (const view of ["front", "side", "top"] as const) {
        controls.alignOrthographic(view, model);
        for (let time = 0; time <= 320; time += 40) {
          frames.shift()?.(time);
          for (const point of grid.positions) {
            const depth = point.clone().sub(entity.getPosition()).dot(entity.forward);
            expect(depth).toBeGreaterThan(camera.nearClip);
            expect(depth).toBeLessThan(camera.farClip);
          }
        }
        expect(frames).toHaveLength(0);
      }
    } finally {
      controls.destroy();
      request.mockRestore();
      cancel.mockRestore();
      clock.mockRestore();
    }
  });

  it.each(["front", "side"] as const)("keeps the %s orthographic framing continuous when orbiting", (view) => {
    const { canvas, camera, controls, onOrthographicViewChange } = setup();
    controls.alignOrthographic(view, new BoundingBox(new Vec3(), new Vec3(2, 1, 3)), 0.85, false);
    const aligned = controls.snapshot();
    expect(camera.projection).toBe(PROJECTION_ORTHOGRAPHIC);
    expect(onOrthographicViewChange).toHaveBeenLastCalledWith(view);

    canvas.dispatchEvent(pointerEvent("pointerdown", 40, 40));
    canvas.dispatchEvent(pointerEvent("pointermove", 60, 48));
    canvas.dispatchEvent(pointerEvent("pointerup", 60, 48));

    const orbited = controls.snapshot();
    expect(camera.projection).toBe(PROJECTION_ORTHOGRAPHIC);
    expect(orbited.projection).toBe(PROJECTION_ORTHOGRAPHIC);
    expect(orbited.orthoHeight).toBe(aligned.orthoHeight);
    expect(orbited.distance).toBe(aligned.distance);
    expect(orbited.orthographicView).toBeNull();
    expect(onOrthographicViewChange).toHaveBeenLastCalledWith(null);
    controls.destroy();
  });

  it("leaves the top view through a stable pole without flipping the screen direction", () => {
    const { canvas, camera, controls, lookAt, onOrthographicViewChange } = setup();
    controls.alignOrthographic("top", new BoundingBox(new Vec3(), new Vec3(2, 1, 3)), 0.85, false);
    const aligned = controls.snapshot();

    canvas.dispatchEvent(pointerEvent("pointerdown", 40, 40));
    canvas.dispatchEvent(pointerEvent("pointermove", 60, 40));
    canvas.dispatchEvent(pointerEvent("pointerup", 60, 40));

    const orbited = controls.snapshot();
    const up = lookAt.mock.calls.at(-1)?.[1] as Vec3;
    expect(camera.projection).toBe(PROJECTION_ORTHOGRAPHIC);
    expect(orbited.orthographicView).toBeNull();
    expect(orbited.orthoHeight).toBe(aligned.orthoHeight);
    expect(orbited.distance).toBe(aligned.distance);
    expect(orbited.pitch).toBeCloseTo(89.9);
    expect(up.z).toBeLessThan(-0.99);
    expect(onOrthographicViewChange).toHaveBeenLastCalledWith(null);
    controls.destroy();
  });

});
