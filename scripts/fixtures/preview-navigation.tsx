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
  mockIPC(() => undefined, { shouldMockEvents: true });
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
  const frameTimes: Array<{ frame: number; wall: number }> = [];
  let timingFrame = 0;
  const observeFrame = (frame: number) => { frameTimes.push({ frame, wall: performance.now() }); timingFrame = requestAnimationFrame(observeFrame); };
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
  assert(flyDistance <= flyMaximum + 0.00001, `Flight exceeded the zoom-relative speed: ${flyDistance} > ${flyMaximum}; height=${flightHeight}, start=${flyStarted}, seconds=${flySeconds}, frames=${JSON.stringify(frameTimes)}`);
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
  const proof = { renderer, locale, viewportWidth: innerWidth, viewportHeight: innerHeight, positionError, directionDot, focusReturned: true, immediateFlight: true,
    panDistance: moved, expectedPanDistance: expected, flyDistance, flySeconds, flyMaximum, orthographicReturns: checks, buttons };
  await fetch("/proof", { method: "POST", body: JSON.stringify(proof) });
}
void verify().catch(error => fetch("/failed", { method: "POST", body: String(error) }));
