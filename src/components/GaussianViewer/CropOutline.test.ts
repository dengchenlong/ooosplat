import { EventHandler, type Application } from "playcanvas";
import { describe, expect, it, vi } from "vitest";
import { buildCropOutline, CropOutline } from "./CropOutline";

describe("buildCropOutline", () => {
  it("queues lines only for actual render frames and releases its listener", () => {
    const app = Object.assign(new EventHandler(), { drawLines: vi.fn(), scene: { layers: { getLayerById: () => undefined } } });
    const outline = new CropOutline(app as unknown as Application);
    outline.setCrop({ kind: "box", center: [0, 0, 0], size: [1, 1, 1] }); outline.setVisible(true);
    for (let i = 0; i < 1000; i++) app.fire("update");
    expect(app.drawLines).not.toHaveBeenCalled();
    app.fire("prerender"); expect(app.drawLines).toHaveBeenCalledTimes(1);
    outline.destroy(); outline.destroy(); app.fire("prerender");
    expect(app.drawLines).toHaveBeenCalledTimes(1);
  });
  it("builds only the twelve edges of an axis-aligned box", () => {
    const geometry = buildCropOutline({ kind: "box", center: [1, 2, 3], size: [4, 6, 8] });

    expect(geometry.positions).toHaveLength(24);
    expect(geometry.colors).toHaveLength(24);
    expect(geometry.positions[0].toArray()).toEqual([-1, -1, -1]);
    expect(geometry.positions[1].toArray()).toEqual([3, -1, -1]);
  });

  it("builds three great circles for a sphere without gizmo geometry", () => {
    const geometry = buildCropOutline({ kind: "sphere", center: [2, 3, 4], radius: 5 });

    expect(geometry.positions).toHaveLength(3 * 72 * 2);
    expect(geometry.colors).toHaveLength(geometry.positions.length);
    expect(geometry.positions[0].toArray()).toEqual([7, 3, 4]);
  });
});
