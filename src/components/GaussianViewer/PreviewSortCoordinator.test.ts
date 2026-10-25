import { GraphNode, EventHandler, Mat4, type Application, type CameraComponent } from "playcanvas";
import { GSplatUnifiedSorter } from "playcanvas/build/playcanvas/src/scene/gsplat-unified/gsplat-unified-sorter.js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PreviewSortCoordinator } from "./PreviewSortCoordinator";

class WorkerFixture extends EventTarget {
  requests: Array<{ order: ArrayBuffer; version: number; params: object }> = [];
  version = 1;
  terminated = false;
  on(_event: string, callback: (event: MessageEvent) => void) { this.addEventListener("message", event => callback(event as MessageEvent)); }
  postMessage(message: { command: string; version?: number; order: ArrayBuffer; sortParams: object }) {
    if (message.command === "intervals") this.version = message.version!;
    if (message.command === "sort") this.requests.push({ order: message.order, version: this.version, params: message.sortParams });
  }
  complete() {
    const request = this.requests.shift()!;
    this.dispatchEvent(new MessageEvent("message", { data: { order: request.order, version: request.version, count: 1, sortTime: 1 } }));
  }
  terminate() { this.terminated = true; }
}

function fixture(existingJob = false, forceFrames?: () => () => void) {
  const app = new EventHandler() as EventHandler & { renderNextFrame: boolean; renderer: object; scene: EventHandler & { gsplat: { radialSorting: boolean } } };
  app.scene = Object.assign(new EventHandler(), { gsplat: { radialSorting: false } });
  const node = new GraphNode("camera");
  const model = new GraphNode("model");
  const camera = { entity: node, projectionMatrix: new Mat4() } as CameraComponent;
  Object.assign(node, { camera });
  const world = { currentVersion: 0, lastWorldStateVersion: 1, awaitingLodUpdate: false, pendingLoadCount: 0, getState: () => ({ splats: [{ node: model }] }) };
  const sorter = new GSplatUnifiedSorter(app.scene as never);
  const manager = { cpuSorter: sorter, cameraNode: node, world, sortNeeded: true, createSorter: () => sorter };
  sorter.on("sorted", (_count: number, version: number) => { world.currentVersion = version; });
  const layer = { gsplatManager: manager, gsplatManagerShadow: null };
  const data = { layersMap: new Map([[1, layer]]), getLayerData: () => layer };
  app.renderer = { gsplatDirector: { camerasMap: new Map([[camera, data]]), getCameraData: () => data } };
  sorter.setSortParameters({ command: "intervals", textureSize: 1, version: 1 });
  if (existingJob) sorter.setSortParams([{ x: -1 }], false);
  const coordinator = new PreviewSortCoordinator(app as unknown as Application, forceFrames);
  const worker = sorter.worker as unknown as WorkerFixture;
  const render = () => {
    sorter.applyPendingSorted();
    if (manager.sortNeeded) {
      sorter.setSortParams([{ x: node.getPosition().x }], false);
      manager.sortNeeded = false; // The engine clears this even when the sorter is busy.
    }
    app.fire("postrender");
  };
  return { app, camera, node, model, world, manager, sorter, worker, coordinator, render };
}

describe("application-local latest-view sorting", () => {
  const resources: ReturnType<typeof fixture>[] = [];
  beforeEach(() => vi.stubGlobal("Worker", WorkerFixture));
  afterEach(() => {
    for (const f of resources.splice(0)) { f.coordinator.destroy(); f.sorter.destroy(); }
    vi.unstubAllGlobals(); vi.useRealTimers();
  });
  const create = (existingJob = false) => { const f = fixture(existingJob); resources.push(f); return f; };

  it("releases forced rendering on a ready frame, cancellation, timeout and scene destruction", async () => {
    vi.useFakeTimers();
    const releases: ReturnType<typeof vi.fn>[] = [];
    const forceFrames = vi.fn(() => { const release = vi.fn(); releases.push(release); return release; });
    const f = fixture(false, forceFrames); resources.push(f);
    const ready = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout");
    f.render(); f.worker.complete(); f.render(); await ready;
    expect(releases[0]).toHaveBeenCalledTimes(1);
    const cancel = new AbortController();
    const cancelled = f.coordinator.waitForFrame(f.camera, cancel.signal, "timeout");
    cancel.abort(); await expect(cancelled).rejects.toMatchObject({ name: "AbortError" });
    expect(releases[1]).toHaveBeenCalledTimes(1);
    const timed = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout", 10);
    const rejected = expect(timed).rejects.toThrow("timeout");
    await vi.advanceTimersByTimeAsync(10); await rejected;
    expect(releases[2]).toHaveBeenCalledTimes(1);
    const destroyed = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout");
    f.coordinator.destroy(); await expect(destroyed).rejects.toMatchObject({ name: "AbortError" });
    expect(releases[3]).toHaveBeenCalledTimes(1);
  });

  it("reissues a restored view dropped by a busy real PlayCanvas sorter, and waits for application plus rendering", async () => {
    const f = create();
    f.render(); // Initial default-view sort is still running.
    f.node.setPosition(10, 0, 0);
    const done = vi.fn();
    const wait = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout").then(done);
    f.render();
    expect(f.worker.requests).toHaveLength(1);
    f.app.fire("frame:ready", f.camera, null, true, 0);
    f.worker.complete();
    expect(f.worker.requests[0].params).toEqual([{ x: 10 }]);
    f.render();
    await Promise.resolve();
    expect(done).not.toHaveBeenCalled();
    f.worker.complete();
    await Promise.resolve();
    expect(done).not.toHaveBeenCalled(); // Worker completion alone is insufficient.
    f.render();
    await wait;
    expect(done).toHaveBeenCalledOnce();
  });

  it("coalesces fast camera changes, includes model transforms and preserves new world versions", async () => {
    const f = create(); f.render();
    for (const x of [1, 2, 3]) { f.node.setPosition(x, 0, 0); f.manager.sortNeeded = true; f.render(); }
    f.worker.complete();
    expect(f.worker.requests).toHaveLength(1);
    expect(f.worker.requests[0].params).toEqual([{ x: 3 }]);
    f.model.setLocalScale(2, 2, 2);
    f.world.lastWorldStateVersion = 2;
    f.sorter.setSortParameters({ command: "intervals", textureSize: 1, version: 2 });
    const wait = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout");
    f.render();
    expect(f.worker.requests).toHaveLength(2);
    f.worker.complete(); f.render();
    expect(f.world.currentVersion).toBe(1);
    f.worker.complete(); f.render();
    await wait;
    expect(f.world.currentVersion).toBe(2);
  });

  it("isolates applications and cancels waits, restores instance methods, and ignores late replies", async () => {
    const first = create(), second = create();
    const method = first.sorter.setSortParams;
    const signal = new AbortController();
    const wait = first.coordinator.waitForFrame(first.camera, signal.signal, "timeout");
    first.render(); second.render();
    first.coordinator.destroy(); first.coordinator.destroy();
    await expect(wait).rejects.toMatchObject({ name: "AbortError" });
    expect(first.sorter.setSortParams).not.toBe(method);
    first.worker.complete();
    const secondWait = second.coordinator.waitForFrame(second.camera, new AbortController().signal, "timeout");
    second.render(); second.worker.complete(); second.render(); second.worker.complete(); second.render();
    await secondWait;
    const aborted = new AbortController(); aborted.abort();
    await expect(second.coordinator.waitForFrame(second.camera, aborted.signal, "timeout")).rejects.toMatchObject({ name: "AbortError" });
  });

  it("rejects a sort timeout without pretending the stale frame is ready", async () => {
    vi.useFakeTimers();
    const f = create();
    const wait = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "sort timed out");
    const assertion = expect(wait).rejects.toThrow("sort timed out");
    f.render();
    await vi.advanceTimersByTimeAsync(8_000);
    await assertion;
  });

  it("does not mistake an existing in-flight reply for a newly accepted world-version request", async () => {
    const f = create(true);
    f.world.lastWorldStateVersion = 2;
    f.sorter.setSortParameters({ command: "intervals", textureSize: 1, version: 2 });
    const done = vi.fn();
    const wait = f.coordinator.waitForFrame(f.camera, new AbortController().signal, "timeout").then(done);
    f.render(); f.worker.complete(); f.render();
    await Promise.resolve();
    expect(done).not.toHaveBeenCalled();
    f.worker.complete(); f.render(); await wait;
    expect(done).toHaveBeenCalledOnce();
  });
});
