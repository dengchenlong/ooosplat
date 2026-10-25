import type { Application, CameraComponent, Entity } from "playcanvas";
import { requestPreviewRender } from "./PreviewRenderScheduler";

type Director = NonNullable<Application["renderer"]["gsplatDirector"]>;
type CameraData = ReturnType<Director["getCameraData"]>;
type LayerData = ReturnType<CameraData["getLayerData"]>;
type Manager = NonNullable<LayerData["gsplatManager"]>;
type Sorter = NonNullable<Manager["cpuSorter"]>;
type Request = { sequence: number; version: number; view: string; params: object; radial: boolean };
type Tracking = {
  manager: Manager; sorter: Sorter; desired: Request | null; applied: Request | null;
  received: Request | null; pending: Request | null; sent: Array<Request | null>; disposed: boolean;
};

const cancelled = () => new DOMException("Preview sorting cancelled", "AbortError");

/** Application-local compatibility bridge for PlayCanvas 2.21.4's CPU sorter.
 * A busy sorter can silently drop a camera request, while its manager still advances
 * the last-camera cache. Keep that request, and acknowledge only applied results.
 * No Gaussian data or order buffers are copied by this bridge.
 */
export class PreviewSortCoordinator {
  private sequence = 0;
  private disposed = false;
  private trackers = new Set<Tracking>();
  private managers = new WeakSet<Manager>();
  private cameraData = new WeakSet<CameraData>();
  private restorers: Array<() => void> = [];
  private waits = new Set<(error: Error) => void>();

  constructor(private readonly app: Application, private readonly forceFrames: () => () => void = () => () => {}) {
    const director = app.renderer.gsplatDirector;
    if (!director) throw new Error("The Gaussian sorting system is unavailable");
    const original = director.getCameraData;
    director.getCameraData = (...args) => {
      const data = original.apply(director, args);
      this.attachCamera(data);
      return data;
    };
    this.restorers.push(() => { director.getCameraData = original; });
    for (const data of director.camerasMap.values()) this.attachCamera(data);
    const destroyed = app.on("destroy", () => this.destroy());
    this.restorers.push(() => destroyed.off());
  }

  private attachCamera(data: CameraData) {
    if (this.cameraData.has(data)) return;
    this.cameraData.add(data);
    const original = data.getLayerData;
    data.getLayerData = (...args) => {
      const layer = original.apply(data, args);
      this.attachLayer(layer);
      return layer;
    };
    this.restorers.push(() => { data.getLayerData = original; });
    for (const layer of data.layersMap.values()) this.attachLayer(layer);
  }

  private attachLayer(layer: LayerData) {
    for (const manager of [layer.gsplatManager, layer.gsplatManagerShadow]) {
      if (!manager || this.managers.has(manager)) continue;
      this.managers.add(manager);
      const original = manager.createSorter;
      manager.createSorter = () => {
        const sorter = original.call(manager);
        this.attachSorter(manager, sorter);
        return sorter;
      };
      this.restorers.push(() => { manager.createSorter = original; });
      if (manager.cpuSorter) this.attachSorter(manager, manager.cpuSorter);
    }
  }

  private view(manager: Manager) {
    const camera = (manager.cameraNode as Entity).camera!;
    const state = manager.world.getState(manager.world.lastWorldStateVersion);
    return [
      ...manager.cameraNode.getWorldTransform().data,
      ...camera.projectionMatrix.data,
      ...(state?.splats.flatMap(splat => Array.from(splat.node.getWorldTransform().data)) ?? []),
      Number(this.app.scene.gsplat.radialSorting),
    ].join(",");
  }

  private attachSorter(manager: Manager, sorter: Sorter) {
    const tracking: Tracking = { manager, sorter, desired: null, applied: null, received: null, pending: null, sent: Array<Request | null>(sorter.jobsInFlight).fill(null), disposed: false };
    this.trackers.add(tracking);
    const setParams = sorter.setSortParams;
    const setVersion = sorter.setSortParameters;
    const destroy = sorter.destroy;
    const send = (request: Request) => {
      const before = sorter.jobsInFlight;
      setParams.call(sorter, request.params, request.radial);
      if (sorter.jobsInFlight > before) {
        tracking.sent.push(request);
        tracking.pending = null;
      } else {
        tracking.pending = request;
      }
    };
    sorter.setSortParams = (params, radial) => {
      if (this.disposed || tracking.disposed) return;
      const request = { sequence: ++this.sequence, version: manager.world.lastWorldStateVersion, view: this.view(manager), params, radial };
      tracking.desired = request;
      send(request);
    };
    sorter.setSortParameters = payload => {
      // A new world version supersedes any queued camera-only request. Let the
      // engine send its new intervals/version through the existing path.
      tracking.pending = null;
      setVersion.call(sorter, payload);
    };
    const message = () => {
      if (this.disposed || tracking.disposed) return;
      // Registered after PlayCanvas's message listener: jobsInFlight and
      // pendingSorted have already been updated. Worker results are FIFO.
      tracking.received = tracking.sent.shift() ?? null;
      const pending = tracking.pending;
      if (pending && sorter.jobsInFlight === 0) {
        if (pending.version === manager.world.lastWorldStateVersion && pending.view === this.view(manager)) send(pending);
        else { tracking.pending = null; manager.sortNeeded = true; }
      }
      requestPreviewRender(this.app);
    };
    sorter.worker.addEventListener("message", message);
    const applied = sorter.on("sorted", () => { tracking.applied = tracking.received; });
    const release = () => {
      if (tracking.disposed) return;
      tracking.disposed = true;
      tracking.pending = null;
      tracking.sent.length = 0;
      sorter.worker?.removeEventListener("message", message);
      applied.off();
      this.trackers.delete(tracking);
      sorter.setSortParams = setParams;
      sorter.setSortParameters = setVersion;
      sorter.destroy = destroy;
    };
    sorter.destroy = () => { release(); destroy.call(sorter); };
    this.restorers.push(release);
  }

  waitForFrame(camera: CameraComponent, signal: AbortSignal, timeoutMessage: string, timeoutMs = 8_000) {
    if (this.disposed || signal.aborted) return Promise.reject(cancelled());
    const floor = this.sequence;
    const releaseFrames = this.forceFrames();
    const request = () => {
      for (const tracking of this.trackers) {
        if (tracking.manager.cameraNode === camera.entity) tracking.manager.sortNeeded = true;
      }
      requestPreviewRender(this.app);
    };
    request();
    return new Promise<void>((resolve, reject) => {
      let settled = false;
      const finish = (error?: Error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        signal.removeEventListener("abort", abort);
        rendered.off();
        this.waits.delete(fail);
        releaseFrames();
        if (error) reject(error); else resolve();
      };
      const fail = (error: Error) => finish(error);
      const abort = () => finish(cancelled());
      const rendered = this.app.on("postrender", () => {
        const active = [...this.trackers].filter(t => t.manager.cameraNode === camera.entity && t.manager.cpuSorter === t.sorter);
        const ready = active.length > 0 && active.every(t => {
          const { world } = t.manager;
          return t.applied && t.desired && t.applied.sequence > floor
            && t.applied.sequence === t.desired.sequence && !t.pending
            && t.applied.view === this.view(t.manager)
            && t.applied.version === world.currentVersion && world.currentVersion === world.lastWorldStateVersion
            && !world.awaitingLodUpdate && world.pendingLoadCount === 0;
        });
        if (ready) finish();
        else {
          // Managers may have been created during this first render.
          for (const t of active) {
            if (!t.desired || t.desired.sequence <= floor || t.desired.view !== this.view(t.manager)
              || t.desired.version !== t.manager.world.lastWorldStateVersion) t.manager.sortNeeded = true;
          }
          requestPreviewRender(this.app);
        }
      });
      const timer = setTimeout(() => finish(new Error(timeoutMessage)), timeoutMs);
      signal.addEventListener("abort", abort, { once: true });
      this.waits.add(fail);
    });
  }

  destroy() {
    if (this.disposed) return;
    this.disposed = true;
    for (const fail of [...this.waits]) fail(cancelled());
    for (const restore of this.restorers.reverse()) restore();
    this.restorers.length = 0;
  }
}
