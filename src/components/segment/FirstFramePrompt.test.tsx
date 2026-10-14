// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FirstFramePrompt } from "./FirstFramePrompt";
import type { FramePrompt } from "../../lib/sam2/prompt";

let container: HTMLDivElement;
let root: Root;
let rectMock: ReturnType<typeof vi.spyOn>;

function mouseEvent(type: string, x: number, y: number, alt = false): MouseEvent {
  return new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    button: 0,
    altKey: alt,
  });
}

async function renderFirstFrame(onChange?: (prompt: FramePrompt) => void) {
  await act(async () => {
    root.render(
      <FirstFramePrompt
        frameId="0000"
        frameUrl="file:///frames/0000.png"
        frameWidth={64}
        frameHeight={48}
        objectId="bracelet"
        onChange={onChange}
      />,
    );
  });
}

function overlay(): HTMLElement {
  return container.querySelector("[data-testid='prompt-overlay']") as HTMLElement;
}

describe("FirstFramePrompt", () => {
  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    // jsdom has no layout: present the frame as a 200x100 box at origin.
    rectMock = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue({
        width: 200,
        height: 100,
        left: 0,
        top: 0,
        right: 200,
        bottom: 100,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      } as DOMRect);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    rectMock.mockRestore();
  });

  it("adds an include point on click, scaled to native frame pixels", async () => {
    const prompts: FramePrompt[] = [];
    await renderFirstFrame((p) => prompts.push(p));
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("click", 100, 50));
    });
    expect(prompts).toHaveLength(1);
    expect(prompts[0]).toEqual({
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [{ x: 32, y: 24, label: 1 }],
      box: null,
    });
  });

  it("adds an exclude point with Alt held", async () => {
    const prompts: FramePrompt[] = [];
    await renderFirstFrame((p) => prompts.push(p));
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("click", 20, 20, true));
    });
    expect(prompts[0].points).toEqual([{ x: 6.4, y: 9.6, label: 0 }]);
  });

  it("drag in box mode commits a normalized box", async () => {
    const prompts: FramePrompt[] = [];
    await renderFirstFrame((p) => prompts.push(p));
    await act(async () => {
      (container.querySelector("[data-testid='mode-box']") as HTMLElement).click();
    });
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("mousedown", 20, 20));
      overlay().dispatchEvent(mouseEvent("mousemove", 120, 70));
      overlay().dispatchEvent(mouseEvent("mouseup", 120, 70));
    });
    expect(prompts[0].points).toEqual([]);
    expect(prompts[0].box).toEqual([6.4, 9.6, 38.4, 33.6]);
  });

  it("undo removes the last point and clear empties the prompt", async () => {
    const prompts: FramePrompt[] = [];
    await renderFirstFrame((p) => prompts.push(p));
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("click", 10, 10));
      overlay().dispatchEvent(mouseEvent("click", 40, 20));
    });
    expect(prompts.at(-1)?.points).toHaveLength(2);

    await act(async () => {
      (container.querySelector("[data-testid='undo-point']") as HTMLElement).click();
    });
    expect(prompts.at(-1)?.points).toHaveLength(1);

    await act(async () => {
      (container.querySelector("[data-testid='clear-prompt']") as HTMLElement).click();
    });
    expect(prompts.at(-1)?.points).toEqual([]);
  });

  it("switches between point and box modes via the toolbar", async () => {
    const prompts: FramePrompt[] = [];
    await renderFirstFrame((p) => prompts.push(p));
    await act(async () => {
      (container.querySelector("[data-testid='mode-box']") as HTMLElement).click();
    });
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("click", 50, 50));
    });
    expect(prompts).toHaveLength(0); // box mode ignores plain clicks
    await act(async () => {
      (container.querySelector("[data-testid='mode-point']") as HTMLElement).click();
    });
    await act(async () => {
      overlay().dispatchEvent(mouseEvent("click", 50, 50));
    });
    expect(prompts).toHaveLength(1);
    expect(prompts[0].points).toHaveLength(1);
  });
});
