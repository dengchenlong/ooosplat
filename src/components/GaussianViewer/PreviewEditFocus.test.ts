// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PreviewEditFocus } from "./PreviewEditFocus";

let canvas: HTMLCanvasElement, button: HTMLButtonElement, input: HTMLInputElement;
let focus: PreviewEditFocus;
let allowed: boolean;
beforeEach(() => {
  canvas = document.createElement("canvas"); canvas.tabIndex = 0;
  button = document.createElement("button"); input = document.createElement("input");
  document.body.append(canvas, button, input); button.focus(); allowed = true;
  focus = new PreviewEditFocus(() => { canvas.focus({ preventScroll: true }); return document.activeElement === canvas; }, () => allowed);
});
afterEach(() => { focus.destroy(); canvas.remove(); button.remove(); input.remove(); });

it.each(["transform", "rectangle", "crop", "undo", "redo", "save", "speed"])("returns canvas focus after %s without scrolling", () => {
  const spy = vi.spyOn(canvas, "focus");
  focus.begin()();
  expect(document.activeElement).toBe(canvas);
  expect(spy).toHaveBeenCalledWith({ preventScroll: true });
});
it("waits for the actual edit lock to clear, then focuses only once", () => {
  const spy = vi.spyOn(canvas, "focus");
  const finish = focus.begin(); allowed = false; finish();
  expect(document.activeElement).toBe(button);
  allowed = true; focus.flush(); focus.flush();
  expect(document.activeElement).toBe(canvas); expect(spy).toHaveBeenCalledOnce();
});
it.each(["input", "pointer", "window", "visibility", "new-operation", "project"])("ignores late completion after %s", reason => {
  const finish = focus.begin();
  if (reason === "input") input.focus();
  if (reason === "pointer") document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
  if (reason === "window") window.dispatchEvent(new Event("blur"));
  if (reason === "visibility") document.dispatchEvent(new Event("visibilitychange"));
  if (reason === "new-operation") focus.begin();
  if (reason === "project") focus.destroy();
  finish(); focus.flush();
  expect(document.activeElement).not.toBe(canvas);
});
it("does not steal focus when a dialog appears while waiting to unlock", () => {
  allowed = false; focus.begin()(); input.focus();
  allowed = true; focus.flush();
  expect(document.activeElement).toBe(input);
});
it("allows explicit input confirmation after blur without a delayed timer", () => {
  input.focus(); const finish = focus.begin(); input.blur(); finish();
  expect(document.activeElement).toBe(canvas);
});
