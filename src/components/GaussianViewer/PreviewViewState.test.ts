// @vitest-environment jsdom
import { PROJECTION_ORTHOGRAPHIC, PROJECTION_PERSPECTIVE } from "playcanvas";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { readPreviewView, savePreviewView } from "./PreviewViewState";
import type { ViewerCameraState } from "./ViewerControls";

const view: ViewerCameraState = {
  target: [1, 2, 3], yaw: 47, pitch: 24, distance: 8,
  horizontalFrameOffset: 0.2, projection: PROJECTION_PERSPECTIVE,
  orthoHeight: 10, orthographicView: null,
};

describe("preview view persistence", () => {
  beforeEach(() => { localStorage.clear(); vi.restoreAllMocks(); });

  it("restores separate project views including orthographic framing", () => {
    const side = { ...view, projection: PROJECTION_ORTHOGRAPHIC, orthographicView: "side" as const, orthoHeight: 4 };
    expect(savePreviewView("first", view)).toBe(true);
    savePreviewView("second", side);
    expect(readPreviewView("first")).toEqual(view);
    expect(readPreviewView("second")).toEqual(side);
    expect(readPreviewView("unknown")).toBeNull();
  });

  it("ignores malformed, nonfinite, and incompatible saved state without breaking controls", () => {
    for (const value of ["broken", JSON.stringify({ version: 2, state: view }), JSON.stringify({ version: 1, state: { ...view, distance: -1 } })]) {
      localStorage.setItem("ooo-splat-preview-view-v1:first", value);
      expect(readPreviewView("first")).toBeNull();
    }
    expect(savePreviewView("invalid", { ...view, target: [NaN, 2, 3] })).toBe(false);
    expect(readPreviewView("invalid")).toBeNull();
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Storage unavailable"); });
    expect(savePreviewView("first", view)).toBe(false);
  });
});
