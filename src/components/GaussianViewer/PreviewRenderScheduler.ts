import type { Application } from "playcanvas";

const schedulers = new WeakMap<Application, PreviewRenderScheduler>();
const FRAME_MS = 1000 / 60;

/** Also works before the scene effect installs its scheduler. */
export function requestPreviewRender(app: Application) {
  const scheduler = schedulers.get(app);
  if (scheduler) scheduler.request();
  else app.renderNextFrame = true;
}

/** Keeps the engine's update/input loop, but submits GPU work only when dirty.
 * Explicit generations retain requests made inside postrender (AppBase clears
 * renderNextFrame after that event). No model data or React state lives here.
 */
export class PreviewRenderScheduler {
  private dirty = true;
  private revision = 0;
  private renderingRevision = -1;
  private nextFrameAt = -Infinity;
  private animationAt = performance.now();
  private animating = false;
  private forced = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private readonly handles: Array<{ off(): void }>;
  private readonly previousAutoRender: boolean;

  constructor(private readonly app: Application) {
    this.previousAutoRender = app.autoRender;
    app.autoRender = false;
    schedulers.set(app, this);
    this.handles = [
      app.on("framerender", this.beforeFrame),
      app.on("postrender", this.afterFrame),
      app.on("destroy", () => this.destroy()),
    ];
    document.addEventListener("visibilitychange", this.visibilityChanged);
    this.request();
  }

  get visible() { return !document.hidden; }

  request() {
    if (this.disposed) return;
    this.dirty = true;
    this.revision++;
    this.app.renderNextFrame = true;
  }

  setAnimating(value: boolean) {
    if (this.disposed || this.animating === value) return;
    this.animating = value;
    this.animationAt = performance.now();
    this.request();
  }

  animationDelta() {
    const now = performance.now();
    const delta = this.visible && this.animating ? Math.max(0, now - this.animationAt) / 1000 : 0;
    this.animationAt = now;
    return delta;
  }

  /** Scoped initialization/capture, never a permanent busy-render fallback. */
  forceFrames = () => {
    if (this.disposed) return () => {};
    this.forced++;
    this.request();
    this.scheduleHiddenFrame();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.forced--;
      if (!this.forced && this.timer !== undefined) { clearTimeout(this.timer); this.timer = undefined; }
    };
  };

  private beforeFrame = () => {
    if (this.disposed) return;
    this.dirty ||= this.app.renderNextFrame;
    const now = performance.now();
    const wanted = this.dirty || (this.animating && this.visible);
    const allowed = this.forced > 0 || (this.visible && wanted && now >= this.nextFrameAt - 0.25);
    this.app.renderNextFrame = allowed;
    if (allowed) {
      this.renderingRevision = this.revision;
      // Anchor to the cadence, not GPU submission completion: measuring from
      // postrender would accidentally halve a 60 Hz display to ~30 FPS.
      this.nextFrameAt = this.forced || now - this.nextFrameAt > FRAME_MS
        ? now + FRAME_MS : this.nextFrameAt + FRAME_MS;
    }
  };

  private afterFrame = () => {
    if (this.disposed) return;
    if (this.renderingRevision === this.revision) this.dirty = false;
    this.renderingRevision = -1;
  };

  private visibilityChanged = () => {
    this.animationAt = performance.now();
    if (this.visible) {
      if (this.timer !== undefined) { clearTimeout(this.timer); this.timer = undefined; }
      this.request();
    } else this.scheduleHiddenFrame();
  };

  private scheduleHiddenFrame() {
    if (this.disposed || !this.forced || this.visible || this.timer !== undefined) return;
    // Browsers may entirely suspend RAF in a hidden/minimized window. Only
    // explicit sort waits may use this path; ordinary preview stays asleep.
    this.timer = setTimeout(() => {
      this.timer = undefined;
      if (this.disposed || !this.forced || this.visible) return;
      if (!this.app.graphicsDevice.contextLost) {
        this.renderingRevision = this.revision;
        this.nextFrameAt = performance.now() + FRAME_MS;
        this.app.render();
      }
      this.scheduleHiddenFrame();
    }, FRAME_MS);
  }

  destroy() {
    if (this.disposed) return;
    this.disposed = true;
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.handles.forEach(handle => handle.off());
    document.removeEventListener("visibilitychange", this.visibilityChanged);
    if (schedulers.get(this.app) === this) {
      schedulers.delete(this.app);
      this.app.autoRender = this.previousAutoRender;
    }
  }
}
