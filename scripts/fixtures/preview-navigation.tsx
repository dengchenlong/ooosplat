// Opt-in acceptance check: real production preview + real PLY; only Tauri IPC is stubbed.
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { Application, PROJECTION_ORTHOGRAPHIC, PROJECTION_PERSPECTIVE, Vec3 } from "playcanvas";
import { LanguageProvider } from "../../src/i18n";
import { GaussianViewer } from "../../src/components/GaussianViewer/GaussianViewer";
import { readPreviewView } from "../../src/components/GaussianViewer/PreviewViewState";
import { trainingReferenceInWorld } from "../../src/components/GaussianViewer/TrainingReferenceView";
import { useGaussianTransformStore } from "../../src/stores/gaussianTransformStore";
import type { GaussianPreviewDescriptor } from "../../src/types/pipeline";
import "../../src/styles.css";

const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
function assert(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }
async function waitFor(check: () => boolean, message: string) {
  const start = performance.now();
  while (!check()) { if (performance.now() - start > 60000) throw new Error(message); await sleep(50); }
}

async function verify() {
  const query = new URLSearchParams(location.search);
  const renderer = query.get("renderer") ?? "webgl2";
  const locale = query.get("locale") === "en" ? "en" : "zh-CN";
  if (renderer === "webgl2") Object.defineProperty(navigator, "gpu", { value: undefined, configurable: true });
  localStorage.clear();
  localStorage.setItem("ooo-splat-language", locale);
  localStorage.setItem("ooo-splat-gizmo-mode-v1", "off");
  let videoCancels = 0, videoCommits = 0;
  mockIPC(cmd => {
    if (cmd === "begin_gaussian_video_export") return { exportId: "isolated-preview-video", destinationPath: "test-only.mp4" };
    if (cmd === "cancel_gaussian_video_export") { videoCancels++; return; }
    if (cmd === "commit_gaussian_video_export") { videoCommits++; throw Error("Verification must not publish a video"); }
  }, { shouldMockEvents: true });
  const descriptor = await (await fetch("/descriptor.json")).json() as GaussianPreviewDescriptor;
  useGaussianTransformStore.getState().load({ ...descriptor, assetUrl: "/model.ply", editMaskAssetUrl: null });
  createRoot(document.getElementById("root")!).render(<LanguageProvider><GaussianViewer previewSessionId={1} onExit={() => undefined} onDisposed={() => undefined} pipelineRunning={false} /></LanguageProvider>);
  const locateName = locale === "en" ? "Auto locate view" : "自动定位视角";
  const button = (name: string) => [...document.querySelectorAll("button")].find(b => b.textContent?.trim() === name);
  await waitFor(() => Boolean(button(locateName) && !button(locateName)!.disabled), "Preview did not become ready");
  assert(!button(locateName)!.querySelector('svg'), 'Auto locate view should be text-only');
  const checkHints = () => {
    const hints = document.querySelector('.preview-input-hints')!;
    for (const item of hints.children) {
      assert(item.firstElementChild?.tagName.toLowerCase() === 'svg' && item.children[1]?.tagName === 'KBD', 'Hint must use icon + key + action');
    }
    const roam = hints.querySelector('.preview-roam-hint')!;
    assert(roam.querySelectorAll('kbd').length === 1 && roam.querySelector('kbd')?.textContent === 'WASD', 'WASD hint must be combined');
    assert(roam.textContent === (locale === 'en' ? 'WASDFly' : 'WASD漫游'), 'Unexpected WASD explanation');
    assert(hints.children[0].textContent === (locale === 'en' ? 'LeftRotate' : '左键旋转'), 'Unexpected rotation hint');
  };
  checkHints();
  const app = Application.getApplication();
  assert(app, "No real renderer");
  assert(app.graphicsDevice.deviceType === renderer, `Requested ${renderer}, actual ${app.graphicsDevice.deviceType}`);
  assert(!app.autoRender, "Preview still renders continuously");
  const frameTimes: number[] = [];
  const rendered = app.on("postrender", () => frameTimes.push(performance.now()));
  const idleChecks: Array<{ name: string; durationMs: number; frames: number }> = [];
  const snapshot = async (name: string) => { const response = await fetch(`/snapshot?name=${name}`); assert(response.ok, "Could not capture verification screenshot"); };
  const sorting = () => [...app.renderer.gsplatDirector!.camerasMap.values()].some(camera => [...camera.layersMap.values()].some(layer => {
    const manager = layer.gsplatManager;
    return manager && ((manager.cpuSorter?.jobsInFlight ?? 0) > 0 || manager.world.currentVersion !== manager.world.lastWorldStateVersion
      || manager.world.awaitingLodUpdate || manager.world.pendingLoadCount > 0);
  }));
  const checkIdle = async (name: string, durationMs = 5000) => {
    let quietAt = performance.now(), observedFrames = frameTimes.length;
    await waitFor(() => {
      if (observedFrames !== frameTimes.length || app.renderNextFrame || sorting()) { quietAt = performance.now(); observedFrames = frameTimes.length; }
      return performance.now() - quietAt >= 1000;
    }, `Preview ${name} never settled`);
    const before = frameTimes.length;
    await sleep(durationMs);
    const frames = frameTimes.length - before;
    idleChecks.push({ name, durationMs, frames });
    assert(frames === 0, `Idle ${name} still rendered ${frames} frames`);
  };
  await checkIdle("adjust");
  // Test-only same-model continuous baseline; immediately restore production policy.
  const baselineStart = frameTimes.length;
  app.autoRender = true; await sleep(500); app.autoRender = false;
  const baselineFrames = frameTimes.length - baselineStart;
  assert(baselineFrames > 0, "Continuous baseline did not draw");
  const gizmoToggle = document.querySelector<HTMLButtonElement>('.transform-gizmo-control button')!;
  gizmoToggle.click(); await checkIdle("gizmo"); await snapshot("gizmo"); gizmoToggle.click(); await sleep(100);
  const camera = app.root.findByName("OOOSplat Preview Camera")!;
  const canvas = app.graphicsDevice.canvas;
  // Synthetic PointerEvents cannot acquire native OS pointer capture. Keep capture
  // bookkeeping local in this harness; production pointer handlers remain unchanged.
  const captures = new Set<number>();
  canvas.setPointerCapture = id => { captures.add(id); };
  canvas.releasePointerCapture = id => { captures.delete(id); };
  canvas.hasPointerCapture = id => captures.has(id);
  const saved = () => readPreviewView(descriptor.projectId)!;
  button(locateName)!.focus();
  button(locateName)!.click();
  assert(document.activeElement === canvas, "Locate button did not synchronously return focus to the canvas");
  await waitFor(() => Boolean(saved()), "Locate did not save view");
  await sleep(400);
  const reference = trainingReferenceInWorld(descriptor.referenceView!, descriptor.transform);
  const positionError = camera.getPosition().distance(reference.position);
  const directionDot = camera.forward.dot(reference.forward);
  assert(positionError < 0.001 && directionDot > 0.99999, `Reference pose mismatch: ${positionError}, ${directionDot}`);
  assert(Math.abs(camera.camera!.fov - reference.fov) < 1e-5, "Reference FOV was not retained");
  // No canvas click/focus here: the actual Locate button must enable immediate flight.
  const locatedPosition = camera.getPosition().clone();
  const immediateKey = new KeyboardEvent("keydown", { code: "KeyW", key: "w", bubbles: true, cancelable: true });
  canvas.dispatchEvent(immediateKey);
  assert(immediateKey.defaultPrevented, "WASD was not accepted directly after clicking Locate");
  await waitFor(() => camera.getPosition().distance(locatedPosition) > 0.0001, "WASD after Locate did not move");
  canvas.dispatchEvent(new KeyboardEvent("keyup", { code: "KeyW", bubbles: true }));
  button(locateName)!.focus(); button(locateName)!.click();
  await waitFor(() => camera.getPosition().distance(reference.position) < 0.001, "Second Locate did not restore reference");
  await sleep(400);
  // Exercise the user-reported order: zoom before any orbit, then right-drag.
  for (let i = 0; i < 8; i++) canvas.dispatchEvent(new WheelEvent("wheel", { deltaY: -200, cancelable: true, bubbles: true }));
  const before = saved();
  const rect = canvas.getBoundingClientRect();
  for (const [type, x, buttons] of [["pointerdown", 300, 2], ["pointermove", 340, 2], ["pointerup", 340, 0]] as const) {
    canvas.dispatchEvent(new PointerEvent(type, { pointerId: 1, button: 2, buttons, clientX: rect.left + x, clientY: rect.top + 300, bubbles: true }));
  }
  const moved = new Vec3(...saved().target).sub(new Vec3(...before.target)).length();
  const expected = 40 * 2 * before.distance * Math.tan(camera.camera!.fov * Math.PI / 360) / rect.height / (before.projectionScaleX ?? 1);
  assert(Math.abs(moved - expected) < Math.max(0.00001, expected * 0.001), `Pan scale mismatch ${moved} vs ${expected}`);
  const beforeFly = camera.getPosition().clone();
  const flightHeight = 2 * saved().distance * Math.tan(camera.camera!.fov * Math.PI / 360);
  const flyStarted = performance.now();
  const flightFrameTimes: Array<{ frame: number; wall: number }> = [];
  let timingFrame = 0;
  const observeFrame = (frame: number) => { flightFrameTimes.push({ frame, wall: performance.now() }); timingFrame = requestAnimationFrame(observeFrame); };
  timingFrame = requestAnimationFrame(observeFrame);
  const flyKey = new KeyboardEvent("keydown", { code: "KeyW", key: "w", bubbles: true, cancelable: true });
  canvas.dispatchEvent(flyKey);
  assert(flyKey.defaultPrevented, `WASD not accepted: focus=${document.activeElement?.tagName}, projection=${camera.camera!.projection}`);
  await waitFor(() => camera.getPosition().distance(beforeFly) > 0.0001, "Focused WASD did not move on a rendered frame");
  canvas.dispatchEvent(new KeyboardEvent("keyup", { code: "KeyW", key: "w", bubbles: true }));
  const flySeconds = (performance.now() - flyStarted) / 1000;
  const flyDistance = camera.getPosition().distance(beforeFly);
  cancelAnimationFrame(timingFrame);
  assert(flyDistance > 0, "Focused WASD did not move");
  const flyMaximum = flightHeight * (flySeconds + 0.02);
  assert(flyDistance <= flyMaximum + 0.00001, `Flight exceeded the zoom-relative speed: ${flyDistance} > ${flyMaximum}; height=${flightHeight}, start=${flyStarted}, seconds=${flySeconds}, frames=${JSON.stringify(flightFrameTimes)}`);
  const stopped = camera.getPosition().clone(); await sleep(100);
  assert(camera.getPosition().distance(stopped) < 1e-5, "Flight continued after key release");
  const checks = [];
  for (const name of locale === "en" ? ["Front", "Side", "Top"] : ["正视", "侧视", "顶视"]) {
    button(name)!.click();
    const expectedForward = name === "Front" || name === "正视" ? new Vec3(0, 0, -1)
      : name === "Side" || name === "侧视" ? new Vec3(-1, 0, 0) : new Vec3(0, -1, 0);
    await waitFor(() => camera.forward.dot(expectedForward) > 0.999999, `Did not align ${name}`);
    // Direction can already match while the alignment animation is still
    // translating the camera. Let that transition finish before testing input.
    await sleep(400);
    await checkIdle(name, 250);
    assert(camera.camera!.projection === PROJECTION_ORTHOGRAPHIC, "Fixed view must remain orthographic until Locate is clicked");
    const atRest = camera.getPosition().clone();
    canvas.focus();
    const orthographicKey = new KeyboardEvent("keydown", { code: "KeyW", bubbles: true, cancelable: true });
    canvas.dispatchEvent(orthographicKey);
    assert(!orthographicKey.defaultPrevented, "Orthographic view accepted WASD before Locate");
    await sleep(50); canvas.dispatchEvent(new KeyboardEvent("keyup", { code: "KeyW", bubbles: true }));
    assert(atRest.distance(camera.getPosition()) < 1e-5, "WASD moved an orthographic camera before Locate");
    button(locateName)!.focus(); button(locateName)!.click();
    assert(document.activeElement === canvas, "Orthographic Locate did not return focus");
    await waitFor(() => saved().projection === PROJECTION_PERSPECTIVE && saved().orthographicView === null
      && camera.getPosition().distance(reference.position) < 0.001 && camera.forward.dot(reference.forward) > 0.99999,
      `Locate did not restore the training perspective camera from ${name}`);
    const state = saved();
    assert(Math.abs((state.projectionScaleX ?? 1) - reference.projectionScaleX) < 1e-6, "Reference focal ratio was lost");
    assert(Math.abs(camera.camera!.fov - reference.fov) < 1e-5, "Training FOV was not restored from the fixed view");
    assert(!document.querySelector('.preview-view-control button.active'), "Fixed-view selection was not cleared");
    const hint = document.querySelector('.preview-roam-hint')!;
    assert(hint.getAttribute('aria-disabled') === 'false', "Flight hint did not become available after Locate");
    checkHints();
    const locatedPositionError = camera.getPosition().distance(reference.position);
    const locatedDirectionDot = camera.forward.dot(reference.forward);
    const startFlight = camera.getPosition().clone();
    const key = new KeyboardEvent("keydown", { code: "KeyW", bubbles: true, cancelable: true });
    canvas.dispatchEvent(key);
    assert(key.defaultPrevented, "WASD was not accepted after leaving the fixed view");
    await waitFor(() => camera.getPosition().distance(startFlight) > 0.0001, "WASD did not move after leaving the fixed view");
    canvas.dispatchEvent(new KeyboardEvent("keyup", { code: "KeyW", bubbles: true }));
    checks.push({ name, projection: state.projection, positionError: locatedPositionError, directionDot: locatedDirectionDot, fov: state.fov, immediateFlight: true });
  }
  const buttons = [...document.querySelectorAll(".preview-view-control button")].map(b => {
    const r = b.getBoundingClientRect(); return { text: b.textContent, x: r.x, y: r.y, width: r.width, height: r.height };
  });
  assert(buttons.every(b => b.height > 20 && b.y === buttons[0].y), "View controls wrapped or clipped");
  const left = document.querySelector(".preview-commandbar-left")!.getBoundingClientRect();
  const center = document.querySelector(".preview-commandbar-center")!.getBoundingClientRect();
  const right = document.querySelector(".preview-header-actions")!.getBoundingClientRect();
  assert(left.right <= center.left && center.right <= right.left, "Command-bar groups overlap");
  const header = document.querySelector('.preview-header') ?? document.querySelector('.preview-input-hints')!.parentElement!;
  assert(header.scrollWidth <= header.clientWidth + 1, 'Header tips overflow');
  assert(header.getBoundingClientRect().right <= innerWidth + 1, 'Header is clipped by the browser viewport');
  const cropBefore = useGaussianTransformStore.getState().editing;
  const cropComparisons: Array<{ kind: string; meanByteDifference: number }> = [];
  const canvasPixels = () => {
    const copy = document.createElement("canvas"); copy.width = canvas.width; copy.height = canvas.height;
    const context = copy.getContext("2d")!; context.drawImage(canvas, 0, 0);
    return context.getImageData(0, 0, copy.width, copy.height).data;
  };
  for (const kind of ["sphere", "box"] as const) {
    const crop = kind === "sphere" ? { kind, center: saved().target, radius: saved().distance * .5 }
      : { kind, center: saved().target, size: [saved().distance, saved().distance, saved().distance] as [number, number, number] };
    // Isolated in-memory UI fixture only; no edits are sent to a project.
    useGaussianTransformStore.setState({ tool: kind, editing: { ...cropBefore, crop } });
    await checkIdle(kind);
    await snapshot(kind);
    if (renderer === "webgl2") {
      const idlePixels = canvasPixels();
      app.autoRender = true; await sleep(500); app.autoRender = false;
      const continuousPixels = canvasPixels();
      let error = 0;
      for (let i = 0; i < idlePixels.length; i++) error += Math.abs(idlePixels[i] - continuousPixels[i]);
      const meanByteDifference = error / idlePixels.length;
      cropComparisons.push({ kind, meanByteDifference });
      await snapshot(`${kind}-continuous`);
      assert(meanByteDifference < 1, `On-demand ${kind} differs from continuous rendering: ${meanByteDifference}`);
    }
  }
  useGaussianTransformStore.setState({ tool: "transform", editing: cropBefore }); await sleep(100);
  const animationName = locale === "en" ? "Animation" : "动画";
  const splatComponent = app.root.findComponents("gsplat")[0] as import("playcanvas").GSplatComponent;
  const animationSamples: Array<{ at: number; seconds: number }> = [];
  const animationSampling = app.on("prerender", () => {
    if (document.querySelector(".animation-time")) animationSamples.push({ at: performance.now(), seconds: Number(splatComponent.getParameter("uOoosplatAnimationTime")) });
  });
  button(animationName)!.click();
  await waitFor(() => Boolean(document.querySelector(".animation-time")), "Did not enter Animation");
  const animationFrameStart = frameTimes.length;
  await sleep(1200);
  await waitFor(() => animationSamples.length >= 2, "Animation did not produce two real frames");
  animationSampling.off();
  const firstAnimation = animationSamples[0], lastAnimation = animationSamples.at(-1)!;
  const animationSeconds = lastAnimation.seconds - firstAnimation.seconds;
  const animationWallSeconds = (lastAnimation.at - firstAnimation.at) / 1000;
  const animationFrames = frameTimes.length - animationFrameStart;
  // Sample the actual shader clock at frame submission, not React's throttled
  // HUD: software GL can block a UI commit for longer than a whole frame.
  assert(Math.abs(animationSeconds - animationWallSeconds) < .02, `Animation clock mismatch: ${animationSeconds} vs ${animationWallSeconds}`);
  assert(animationSamples.length <= Math.ceil(animationWallSeconds * 60) + 2, "Animation exceeded 60 FPS");
  document.querySelector<HTMLButtonElement>('.preview-export-menu > button')!.click(); await sleep(50);
  const videoItem = button(locale === "en" ? "Export video" : "导出视频");
  const videoSupported = Boolean(videoItem && !videoItem.disabled);
  let videoFrameProgress = 0;
  if (videoSupported) {
    videoItem!.click();
    await waitFor(() => {
      videoFrameProgress = parseFloat(document.querySelector<HTMLElement>('.video-export-track > span')?.style.width ?? "0");
      const error = document.querySelector('.video-export-error');
      if (error) throw Error(error.textContent ?? "Video export failed");
      return videoFrameProgress > 0;
    }, "Video did not render/encode any frames");
    button(locale === "en" ? "Cancel export" : "取消导出")!.click();
    await waitFor(() => !document.querySelector('.video-export-overlay'), "Video cancel did not restore preview");
    assert(videoCancels > 0 && videoCommits === 0, "Cancelled video was not released or was published");
  } else document.querySelector<HTMLButtonElement>('.preview-export-menu > button')!.click();
  button(locale === "en" ? "Adjust" : "调整")!.click();
  await waitFor(() => Boolean(button(locateName) && !button(locateName)!.disabled), "Did not restore Adjust after Animation/export");
  await checkIdle("after-animation", 250);
  // The real training pose above retains roll. A user orbit must smoothly level
  // it without modifying the world grid/model or zoom.
  button(locateName)!.click(); await sleep(600);
  await snapshot("rolled-reference");
  const rolled = saved();
  const initialRightY = camera.right.y;
  for (const [type, x, buttons] of [["pointerdown", 120, 1], ["pointermove", 140, 1], ["pointerup", 140, 0]] as const) {
    canvas.dispatchEvent(new PointerEvent(type, { pointerId: 2, button: 0, buttons, clientX: rect.left + x, clientY: rect.top + 180, bubbles: true }));
  }
  await sleep(400);
  assert(Math.abs(saved().roll ?? 0) < 1e-6 && Math.abs(camera.right.y) < 1e-5, "Orbit did not level the horizon");
  assert(Math.abs(saved().distance - rolled.distance) < 1e-5 && camera.camera!.fov === rolled.fov, "Leveling changed zoom");
  await checkIdle("after-level", 250);
  await snapshot("leveled");
  const animationTimes = frameTimes.slice(animationFrameStart, animationFrameStart + animationFrames);
  const minimumFrameMs = Math.min(...animationTimes.slice(1).map((t, i) => t - animationTimes[i]));
  rendered.off();
  const proof = { renderer, locale, viewportWidth: innerWidth, viewportHeight: innerHeight, positionError, directionDot, focusReturned: true, immediateFlight: true,
    idleChecks, baselineFrames, baselineMs: 500, minimumFrameMs, initialRightY, leveledRightY: camera.right.y,
    backendAdapter: (app.graphicsDevice as unknown as { unmaskedRenderer?: string }).unmaskedRenderer ?? null,
    animationSeconds, animationWallSeconds, animationFrames, videoSupported, videoFrameProgress, videoCancels, videoCommits,
    cropComparisons,
    panDistance: moved, expectedPanDistance: expected, flyDistance, flySeconds, flyMaximum, orthographicReturns: checks, buttons };
  await fetch("/proof", { method: "POST", body: JSON.stringify(proof) });
}
void verify().catch(error => fetch("/failed", { method: "POST", body: String(error) }));
