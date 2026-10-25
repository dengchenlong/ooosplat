import { type Application } from "playcanvas";
import { describe, expect, it, vi } from "vitest";
import { buildOrbitAxisGuide, OrbitAxisGuide } from "./OrbitAxisGuide";
import type { ViewerControls, ViewerOrbitGuideState } from "./ViewerControls";

const state: ViewerOrbitGuideState = {
  target: [2, 3, 4],
  radius: 10,
  height: 7,
  angleDegrees: 35,
};

describe("OrbitAxisGuide", () => {
  it("builds a vertical axis through the target plus a horizontal orbit and direction marker", () => {
    const geometry = buildOrbitAxisGuide(state, 8);

    expect(geometry.positions.length).toBeGreaterThan(96 * 2);
    expect(geometry.colors).toHaveLength(geometry.positions.length);
    expect(geometry.positions[0].x).toBe(state.target[0]);
    expect(geometry.positions[0].z).toBe(state.target[2]);
    expect(geometry.positions.some((point) => Math.abs(point.y - state.height) < 1e-6)).toBe(true);
    expect(geometry.positions.some((point) => point.x < state.target[0])).toBe(true);
    expect(geometry.positions.some((point) => point.x > state.target[0])).toBe(true);
  });

  it("only draws on rendered frames while visible, caches geometry and releases its listener once", () => {
    const off = vi.fn();
    const drawLines = vi.fn();
    let draw: (() => void) | undefined;
    const worldLayer = { id: 0 };
    const app = {
      scene: { layers: { getLayerById: vi.fn(() => worldLayer) } },
      drawLines,
      on: vi.fn((_event: string, callback: () => void) => {
        draw = callback;
        return { off };
      }),
    } as unknown as Application;
    const controls = { orbitGuideState: vi.fn(() => state) } as unknown as ViewerControls;
    const guide = new OrbitAxisGuide(app, controls, 8);
    expect(app.on).toHaveBeenCalledWith("prerender", expect.any(Function));

    draw?.();
    expect(drawLines).not.toHaveBeenCalled();
    guide.setVisible(true);
    draw?.();
    expect(drawLines).toHaveBeenCalledWith(expect.any(Array), expect.any(Array), false, worldLayer);
    const firstGeometry = drawLines.mock.calls[0][0];
    draw?.(); expect(drawLines.mock.calls[1][0]).toBe(firstGeometry);
    guide.setModelSpan(10); draw?.(); expect(drawLines.mock.calls[2][0]).not.toBe(firstGeometry);
    expect(guide.isVisible).toBe(true);
    guide.setVisible(false);
    draw?.();
    expect(drawLines).toHaveBeenCalledTimes(3);
    guide.destroy();
    guide.destroy();
    expect(off).toHaveBeenCalledTimes(1);
  });
});
