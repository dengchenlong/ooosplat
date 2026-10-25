// @vitest-environment jsdom
import { EventHandler, type Application } from "playcanvas";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PreviewRenderScheduler, requestPreviewRender } from "./PreviewRenderScheduler";

describe("preview render scheduling", () => {
  let now = 0;
  let hidden = false;
  const schedulers: PreviewRenderScheduler[] = [];
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(performance, "now").mockImplementation(() => now);
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
    now = 0; hidden = false;
  });
  afterEach(() => { schedulers.splice(0).forEach(s => s.destroy()); vi.restoreAllMocks(); vi.useRealTimers(); });
  const setup = () => {
    const events = new EventHandler();
    const app = Object.assign(events, { autoRender: true, renderNextFrame: false, graphicsDevice: { contextLost: false }, render: vi.fn(() => {
      app.fire("prerender"); app.fire("postrender");
    }) }) as unknown as Application;
    const scheduler = new PreviewRenderScheduler(app); schedulers.push(scheduler);
    const tick = (at: number) => {
      now = at; app.fire("update", 1 / 144); app.fire("framerender");
      if (app.autoRender || app.renderNextFrame) { app.render(); app.renderNextFrame = false; }
      app.fire("frameend");
    };
    return { app, scheduler, tick };
  };

  it("renders once then sleeps for five seconds, coalescing requests without an idle timer", () => {
    const f = setup(); f.tick(0);
    for (let i = 1; i <= 720; i++) f.tick(i * 5000 / 720);
    expect(f.app.render).toHaveBeenCalledTimes(1);
    for (let i = 0; i < 20; i++) requestPreviewRender(f.app);
    f.tick(5020); expect(f.app.render).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([60, 120, 144])("caps %s Hz requests and retains the last frame", hz => {
    const f = setup();
    for (let frame = 0; frame <= hz; frame++) { requestPreviewRender(f.app); f.tick(frame * 1000 / hz); }
    expect(vi.mocked(f.app.render).mock.calls.length).toBeLessThanOrEqual(61);
    expect(vi.mocked(f.app.render).mock.calls.length).toBeGreaterThanOrEqual(59);
    requestPreviewRender(f.app); f.tick(1001);
    const count = vi.mocked(f.app.render).mock.calls.length;
    f.tick(1020); expect(f.app.render).toHaveBeenCalledTimes(count + 1);
    f.tick(2000); expect(f.app.render).toHaveBeenCalledTimes(count + 1);
  });

  it("retains a request made inside postrender despite AppBase clearing its flag", () => {
    const f = setup();
    const handle = f.app.on("postrender", () => { handle.off(); requestPreviewRender(f.app); });
    f.tick(0); expect(f.app.renderNextFrame).toBe(false);
    f.tick(20); expect(f.app.render).toHaveBeenCalledTimes(2);
    f.tick(40); expect(f.app.render).toHaveBeenCalledTimes(2);
  });

  it("does not halve the display rate by charging GPU submission time to the next frame", () => {
    const f = setup();
    f.app.on("prerender", () => { now += 4; });
    for (let frame = 0; frame <= 60; frame++) { requestPreviewRender(f.app); f.tick(frame * 1000 / 60); }
    expect(f.app.render).toHaveBeenCalledTimes(61);
  });

  it("uses elapsed time for animation and excludes hidden time", () => {
    const f = setup(); f.scheduler.setAnimating(true);
    const deltas: number[] = [];
    f.app.on("prerender", () => deltas.push(f.scheduler.animationDelta()));
    f.tick(0); f.tick(20); f.tick(40);
    expect(deltas.reduce((a, b) => a + b, 0)).toBeCloseTo(.04);
    hidden = true; document.dispatchEvent(new Event("visibilitychange"));
    f.tick(5000); expect(f.app.render).toHaveBeenCalledTimes(3);
    hidden = false; document.dispatchEvent(new Event("visibilitychange"));
    f.tick(5000); f.tick(5020);
    expect(deltas.reduce((a, b) => a + b, 0)).toBeCloseTo(.06);
    f.scheduler.setAnimating(false); f.tick(5040); f.tick(5060);
    expect(f.app.render).toHaveBeenCalledTimes(6);
  });

  it("temporarily permits forced hidden frames and stops their timer on release/destroy", () => {
    const f = setup(); hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
    f.tick(0); expect(f.app.render).not.toHaveBeenCalled();
    const release = f.scheduler.forceFrames();
    vi.advanceTimersByTime(34); expect(f.app.render).toHaveBeenCalledTimes(2);
    release(); release(); vi.advanceTimersByTime(1000);
    expect(f.app.render).toHaveBeenCalledTimes(2);
    f.scheduler.forceFrames(); f.scheduler.destroy(); vi.advanceTimersByTime(1000);
    expect(f.app.render).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("isolates applications, releases hooks and permits the pre-install fallback", () => {
    const first = setup(), second = setup();
    first.tick(0); second.tick(0);
    requestPreviewRender(first.app); first.tick(20); second.tick(20);
    expect(first.app.render).toHaveBeenCalledTimes(2); expect(second.app.render).toHaveBeenCalledTimes(1);
    first.scheduler.destroy(); first.scheduler.destroy();
    expect(first.app.autoRender).toBe(true);
    expect(first.app.hasEvent("framerender")).toBe(false);
    requestPreviewRender(first.app); expect(first.app.renderNextFrame).toBe(true);
    expect(second.app.autoRender).toBe(false);
  });
});
