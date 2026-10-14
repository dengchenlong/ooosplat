import { describe, expect, it } from "vitest";
import {
  type FramePrompt,
  emptyPrompt,
  promptHasSelection,
  toAddPromptRequest,
  validateFramePrompt,
} from "./prompt";

describe("FramePrompt validation", () => {
  it("accepts a point prompt", () => {
    const prompt: FramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [{ x: 32, y: 24, label: 1 }],
      box: null,
    };
    expect(validateFramePrompt(prompt)).toEqual([]);
    expect(promptHasSelection(prompt)).toBe(true);
  });

  it("accepts a box prompt", () => {
    const prompt: FramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [],
      box: [10, 20, 30, 40],
    };
    expect(validateFramePrompt(prompt)).toEqual([]);
  });

  it("rejects a prompt with neither points nor box", () => {
    const prompt = emptyPrompt("bracelet", "0000");
    const errors = validateFramePrompt(prompt);
    expect(errors.length).toBe(1);
    expect(errors[0]).toMatch(/point|box/i);
    expect(promptHasSelection(prompt)).toBe(false);
  });

  it("rejects a box with non-positive extent", () => {
    const prompt: FramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [],
      box: [30, 40, 30, 40],
    };
    expect(validateFramePrompt(prompt).length).toBeGreaterThan(0);
  });
});

describe("toAddPromptRequest", () => {
  it("maps points with labels to the sidecar request shape", () => {
    const prompt: FramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [
        { x: 12.5, y: 20, label: 1 },
        { x: 40, y: 8.25, label: 0 },
      ],
      box: null,
    };
    expect(toAddPromptRequest(prompt, "s1")).toEqual({
      type: "add_prompt",
      sessionId: "s1",
      objectId: "bracelet",
      frameId: "0000",
      points: [[12.5, 20], [40, 8.25]],
      pointLabels: [1, 0],
    });
  });

  it("maps a box prompt", () => {
    const prompt: FramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0002",
      points: [],
      box: [1, 2, 3, 4],
    };
    expect(toAddPromptRequest(prompt, "s1")).toEqual({
      type: "add_prompt",
      sessionId: "s1",
      objectId: "bracelet",
      frameId: "0002",
      box: [1, 2, 3, 4],
    });
  });

  it("maps a combined points+box prompt", () => {
    const prompt: FramePrompt = {
      objectId: "o",
      anchorFrameId: "0001",
      points: [{ x: 5, y: 6, label: 1 }],
      box: [1, 2, 3, 4],
    };
    const req = toAddPromptRequest(prompt, "s1");
    expect(req.points).toEqual([[5, 6]]);
    expect(req.box).toEqual([1, 2, 3, 4]);
  });

  it("throws on an empty selection instead of producing an invalid request", () => {
    const prompt = emptyPrompt("bracelet", "0000");
    expect(() => toAddPromptRequest(prompt, "s1")).toThrow();
  });
});
