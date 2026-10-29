// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LanguageProvider } from "../../i18n";
import { RoamSpeedControl } from "./RoamSpeedControl";
import { readRoamSpeed, saveRoamSpeed } from "./RoamPreferences";

let container: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear(); localStorage.setItem("ooo-splat-language", "zh-CN");
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); localStorage.clear(); });
async function render(disabled = false) {
  const callbacks = { onChange: vi.fn(), onCommit: vi.fn(), onFocusCanvas: vi.fn() };
  await act(async () => root.render(<LanguageProvider><RoamSpeedControl value={1} disabled={disabled} {...callbacks} /></LanguageProvider>));
  return { ...callbacks, slider: container.querySelector("input")! };
}
it("shows the approved label, limits, accessible value and default", async () => {
  const { slider } = await render();
  expect(container.textContent).toContain("漫游速度"); expect(container.querySelector("output")?.textContent).toBe("1.0×");
  expect([slider.min, slider.max, slider.step]).toEqual(["0.1", "5", "0.1"]);
  expect(parseFloat(slider.style.getPropertyValue('--roam-progress'))).toBeCloseTo(100 * .9 / 4.9);
  expect(container.querySelector("label")?.htmlFor).toBe(slider.id);
});
it("applies changes immediately, commits on pointer release and returns canvas focus", async () => {
  const { slider, onChange, onCommit, onFocusCanvas } = await render();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(slider, "2.5");
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(onChange).toHaveBeenCalledWith(2.5); expect(onCommit).not.toHaveBeenCalled();
  await act(async () => slider.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0 })));
  expect(onCommit).toHaveBeenCalled(); expect(onFocusCanvas).toHaveBeenCalledOnce();
});
it("keeps focus for keyboard adjustments and returns it only on Enter", async () => {
  const { slider, onCommit, onFocusCanvas } = await render(); slider.focus();
  await act(async () => slider.dispatchEvent(new KeyboardEvent("keyup", { key: "ArrowRight", bubbles: true })));
  expect(onCommit).toHaveBeenCalledOnce(); expect(onFocusCanvas).not.toHaveBeenCalled(); expect(document.activeElement).toBe(slider);
  await act(async () => slider.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  expect(onFocusCanvas).toHaveBeenCalledOnce();
});
it("disables orthographic/busy controls and provides the confirmed English label", async () => {
  localStorage.setItem("ooo-splat-language", "en");
  const { slider, onFocusCanvas } = await render(true);
  expect(slider.disabled).toBe(true); expect(container.textContent).toContain("Movement speed");
  await act(async () => slider.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0 })));
  expect(onFocusCanvas).not.toHaveBeenCalled();
});
it("persists only valid local preferences and safely falls back for old/broken records", () => {
  expect(readRoamSpeed()).toBe(1);
  for (const value of [.1, 1, 3, 2.4, 4, 4.9, 5]) { saveRoamSpeed(value); expect(readRoamSpeed()).toBe(value); }
  for (const value of [null, "2", -1, 0, 5.1, 1.25, "broken"]) {
    localStorage.setItem("ooo-splat-roam-speed-v1", value === "broken" ? value : JSON.stringify(value));
    expect(readRoamSpeed()).toBe(1);
  }
  saveRoamSpeed(2); saveRoamSpeed(Infinity); expect(readRoamSpeed()).toBe(2);
});
