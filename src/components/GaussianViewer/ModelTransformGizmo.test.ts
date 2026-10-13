// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { Entity, EventHandler, Quat, Vec3, type Application, type CameraComponent } from "playcanvas";
import type { GaussianTransform } from "../../types/pipeline";
import { ModelTransformGizmo } from "./ModelTransformGizmo";
import { modelWorldCenter } from "./ModelTransformMath";
import { readGizmoMode, saveGizmoMode } from "./GizmoPreferences";

vi.mock("playcanvas", async importOriginal => {
  const pc = await importOriginal<typeof import("playcanvas")>();
  class TestEntity extends pc.Entity {
    constructor(name: string) { super(name, { systems: {}, _entityIndex: {} } as unknown as Application); }
  }
  class FakeGizmo extends pc.EventHandler {
    static createLayer(app: Application) { const layer = { id: 987 }; app.scene.layers.insert(layer as never, 0); return layer; }
    static EVENT_RENDERUPDATE = "render:update";
    static EVENT_TRANSFORMSTART = "transform:start";
    static EVENT_TRANSFORMMOVE = "transform:move";
    static EVENT_TRANSFORMEND = "transform:end";
    nodes: Entity[] = [];
    root = new TestEntity("test gizmo");
    intersectShapes = ["arrow:x", "arrow:y", "arrow:z", "plane:x", "disk:x", "disk:y", "disk:z", "disk:f", "sphere:xyz"]
      .map(name => ({ entity: new TestEntity(name), visible: true, disabled: false, meshInstances: [{ material: { destroy: vi.fn() } }] }));
    allShapes = this.intersectShapes;
    mouseButtons = [true, true, true];
    coordSpace = "local";
    size = 1;
    setTheme = vi.fn();
    enabled = false;
    destroy = vi.fn(() => { this.detach(); });
    constructor(camera: CameraComponent, layer: { id: number }) { super(); camera.layers = [...camera.layers, layer.id]; }
    update() {}
    attach(nodes: Entity[]) { this.nodes = nodes; this.enabled = true; }
    detach() { this.nodes = []; this.enabled = false; this.fire("transform:end"); }
  }
  return { ...pc, Entity: TestEntity, Gizmo: FakeGizmo, TransformGizmo: FakeGizmo, TranslateGizmo: FakeGizmo, RotateGizmo: FakeGizmo };
});

describe("model gizmo transactions and lifecycle", () => {
  const controllers: ModelTransformGizmo[] = [];
  afterEach(() => { controllers.splice(0).forEach(controller => controller.destroy()); localStorage.clear(); vi.restoreAllMocks(); });

  function setup() {
    const created: any[] = [];
    const canvas = document.createElement("canvas");
    canvas.hasPointerCapture = vi.fn(() => false);
    canvas.releasePointerCapture = vi.fn();
    const app = Object.assign(new EventHandler(), { root: new Entity("root"), graphicsDevice: { canvas }, scene: { layers: { insert: vi.fn(), remove: vi.fn() } } }) as unknown as Application;
    const camera = { layers: [0, 1] } as unknown as CameraComponent;
    const original: GaussianTransform = { position: [4, 5, 6], rotation: [10, 20, 30], scale: 2 };
    let current: GaussianTransform | null = structuredClone(original);
    let start: GaussianTransform;
    const finish = vi.fn(cancel => { if (cancel) current = structuredClone(start); });
    const beforeStart = vi.fn();
    const change = vi.fn(next => { current = next; });
    const dragging = vi.fn();
    const controller = new ModelTransformGizmo(app, camera, [-2, 3, 4], {
      current: () => current, beforeStart, begin: () => { start = structuredClone(current!); }, change, finish, dragging,
    });
    controllers.push(controller);
    // EventHandler registration is public; capture the native event emitter,
    // while keeping this test independent of graphics-device implementation.
    const on = EventHandler.prototype.on;
    vi.spyOn(EventHandler.prototype, "on").mockImplementation(function (this: EventHandler, name, callback, scope) {
      if (name === "transform:start" && !created.includes(this)) created.push(this);
      return on.call(this, name, callback, scope);
    });
    controller.configure("translate", true);
    return { controller, app, canvas, camera, created, original, beforeStart, change, finish, dragging, get: () => current!, stale: () => { current = null; } };
  }

  it("uses world axes, commits once, ends on blur/pointer cancellation, and ignores late move events", () => {
    const s = setup(), native = s.created[0];
    expect(native.size).toBe(0.9);
    const theme = native.setTheme.mock.calls[0][0];
    expect([theme.guideBase.x.a, theme.guideBase.y.a, theme.guideBase.z.a]).toEqual([0, 0, 0]);
    expect(theme.guideOcclusion).toBe(1);
    expect(theme).not.toHaveProperty("shapeBase"); expect(theme).not.toHaveProperty("shapeHover");
    expect(native.coordSpace).toBe("world"); expect(native.mouseButtons).toEqual([true, false, false]);
    expect(native.intersectShapes.map((shape: any) => shape.entity.name)).toEqual(["arrow:x", "arrow:y", "arrow:z"]);
    const plane = native.allShapes.find((shape: any) => shape.entity.name === "plane:x");
    plane.entity.enabled = true; plane.visible = true;
    s.app.fire("prerender");
    expect(plane.entity.enabled).toBe(false); expect(plane.visible).toBe(false);
    s.canvas.dispatchEvent(new MouseEvent("pointerdown", { button: 0 }));
    expect(s.beforeStart).toHaveBeenCalledOnce();
    native.fire("transform:start");
    native.nodes[0].setPosition(modelWorldCenter(s.original, [-2, 3, 4]).add(new Vec3(3, 0, 0)));
    native.fire("transform:move"); native.fire("transform:move");
    expect(s.get().position[0]).toBeCloseTo(7);
    window.dispatchEvent(new Event("blur"));
    expect(s.finish).toHaveBeenCalledExactlyOnceWith(false);
    native.fire("transform:move"); expect(s.change).toHaveBeenCalledTimes(2);
    expect(s.dragging.mock.calls.map(call => call[0])).toEqual([true, false]);
    native.fire("transform:start"); s.canvas.dispatchEvent(new Event("pointercancel"));
    expect(s.finish).toHaveBeenCalledTimes(2);
    native.fire("transform:start"); s.canvas.dispatchEvent(new Event("lostpointercapture"));
    expect(s.finish).toHaveBeenCalledTimes(3);
  });

  it("rotates around the cached center, restores on Esc/device loss and releases every session resource once", () => {
    const s = setup(), translation = s.created[0];
    s.controller.configure("rotate", true);
    const rotation = s.created[1];
    expect(rotation.size).toBe(0.9);
    const theme = rotation.setTheme.mock.calls[0][0];
    expect([theme.guideBase.x.a, theme.guideBase.y.a, theme.guideBase.z.a]).toEqual([0, 0, 0]);
    expect(theme.guideOcclusion).toBe(1); expect(theme).not.toHaveProperty("shapeHover");
    expect(translation.destroy).toHaveBeenCalledOnce(); expect(s.camera.layers).toEqual([0, 1, 987]);
    expect(rotation.intersectShapes.map((shape: any) => shape.entity.name)).toEqual(["disk:x", "disk:y", "disk:z"]);
    const center = modelWorldCenter(s.original, [-2, 3, 4]);
    rotation.fire("transform:start");
    rotation.nodes[0].setRotation(new Quat().setFromEulerAngles(60, 90, 30)); rotation.fire("transform:move");
    expect(modelWorldCenter(s.get(), [-2, 3, 4]).distance(center)).toBeLessThan(1e-8);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true }));
    expect(s.get()).toEqual(s.original); expect(s.finish).toHaveBeenLastCalledWith(true);
    rotation.fire("transform:start"); s.controller.destroy(true); s.controller.destroy(true);
    expect(s.finish).toHaveBeenCalledTimes(2); expect(rotation.destroy).toHaveBeenCalledOnce();
    expect(s.camera.layers).toEqual([0, 1]); expect(s.app.scene.layers.remove).toHaveBeenCalledOnce();
    const count = s.beforeStart.mock.calls.length;
    s.canvas.dispatchEvent(new MouseEvent("pointerdown", { button: 0 }));
    expect(s.beforeStart).toHaveBeenCalledTimes(count);
    rotation.fire("transform:move"); expect(s.change).toHaveBeenCalledOnce();
  });

  it("disables picking when hidden, does not modify a stale editor, and keeps mode preferences local and optional", () => {
    const s = setup(), native = s.created[0];
    s.controller.configure("translate", false);
    expect(native.enabled).toBe(false); expect(native.nodes).toEqual([]);
    native.fire("transform:start"); expect(s.dragging).not.toHaveBeenCalled();
    s.controller.configure("translate", true); s.stale();
    native.fire("transform:start"); native.fire("transform:move"); expect(s.change).not.toHaveBeenCalled();
    expect(readGizmoMode()).toBe("translate");
    saveGizmoMode("rotate"); expect(readGizmoMode()).toBe("rotate");
    localStorage.setItem("ooo-splat-gizmo-mode-v1", "invalid"); expect(readGizmoMode()).toBe("translate");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Disabled"); });
    expect(() => saveGizmoMode("off")).not.toThrow();
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("Disabled"); });
    expect(readGizmoMode()).toBe("translate");
  });
});
