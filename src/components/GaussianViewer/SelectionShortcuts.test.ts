// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invertSelectionShortcutLabel, isInvertSelectionShortcut } from "./SelectionShortcuts";

describe("invert selection keyboard shortcut", () => {
  beforeEach(() => document.body.replaceChildren());

  it("supports Ctrl+I, Cmd+I and the physical I key across layouts", () => {
    expect(isInvertSelectionShortcut(new KeyboardEvent("keydown", { key: "i", ctrlKey: true }))).toBe(true);
    expect(isInvertSelectionShortcut(new KeyboardEvent("keydown", { key: "I", metaKey: true }))).toBe(true);
    expect(isInvertSelectionShortcut(new KeyboardEvent("keydown", { key: "ш", code: "KeyI", ctrlKey: true }))).toBe(true);
    expect(invertSelectionShortcutLabel("Win32")).toBe("Ctrl+I");
    expect(invertSelectionShortcutLabel("MacIntel")).toBe("⌘+I");
  });

  it("ignores plain typing, repeats, composition and other modifier combinations", () => {
    for (const options of [
      { ctrlKey: false }, { repeat: true }, { isComposing: true },
      { altKey: true }, { shiftKey: true }, { key: "z" },
    ]) {
      expect(isInvertSelectionShortcut(new KeyboardEvent("keydown", { key: "i", ctrlKey: true, ...options }))).toBe(false);
    }
  });

  it("does not invert while an input, selection field or editable text has focus", () => {
    const invert = vi.fn();
    const listener = (event: KeyboardEvent) => { if (isInvertSelectionShortcut(event)) invert(); };
    window.addEventListener("keydown", listener);
    try {
      for (const tag of ["input", "textarea", "select", "div"]) {
        const field = document.createElement(tag);
        field.tabIndex = 0;
        if (tag === "div") field.setAttribute("contenteditable", "true");
        document.body.append(field);
        field.focus();
        field.dispatchEvent(new KeyboardEvent("keydown", { key: "i", ctrlKey: true, bubbles: true }));
        expect(invert).not.toHaveBeenCalled();
        field.remove();
      }
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "i", ctrlKey: true }));
      expect(invert).toHaveBeenCalledTimes(1);
    } finally { window.removeEventListener("keydown", listener); }
  });
});
