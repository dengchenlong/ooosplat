// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectAssetDescriptor } from "../types/pipeline";

const asset = (kind: ProjectAssetDescriptor["kind"]): ProjectAssetDescriptor => ({
  kind,
  path: `C:/proj/${kind === "final" ? "final.ply" : `assets/${kind}.ply`}`,
  fileSize: 1024,
  splatCount: 100,
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("AssetVisibilityPanel", () => {
  it("starts with every asset visible when no toggle has been recorded", async () => {
    const { defaultHiddenKinds } = await import("./AssetVisibilityPanel");
    expect([...defaultHiddenKinds([asset("final"), asset("object"), asset("background")])]).toEqual([]);
  });

  it("renders one checkbox per asset, all checked initially", async () => {
    const { AssetVisibilityPanel } = await import("./AssetVisibilityPanel");
    await act(async () => {
      root.render(
        <AssetVisibilityPanel
          assets={[asset("final"), asset("object"), asset("background")]}
          hiddenKinds={new Set()}
          onHiddenKindsChange={vi.fn()}
        />,
      );
    });
    const checkboxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    expect(checkboxes).toHaveLength(3);
    expect(checkboxes.every((box) => box.checked)).toBe(true);
    expect(checkboxes.map((box) => box.dataset.assetKind)).toEqual(["final", "object", "background"]);
  });

  it("reports only the clicked asset as hidden when toggled off", async () => {
    const { AssetVisibilityPanel } = await import("./AssetVisibilityPanel");
    let hidden: ReadonlySet<string> | null = null;
    await act(async () => {
      root.render(
        <AssetVisibilityPanel
          assets={[asset("final"), asset("object"), asset("background")]}
          hiddenKinds={new Set()}
          onHiddenKindsChange={(next) => {
            hidden = next;
          }}
        />,
      );
    });
    const checkboxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    await act(async () => checkboxes[1]?.click());
    expect(hidden).not.toBeNull();
    expect([...hidden!]).toEqual(["object"]);
  });

  it("unhides the clicked asset again when toggled twice", async () => {
    const { AssetVisibilityPanel } = await import("./AssetVisibilityPanel");
    let hidden: ReadonlySet<string> = new Set(["object"]);
    const rerender = async (nextHidden: ReadonlySet<string>) => {
      root.render(
        <AssetVisibilityPanel
          assets={[asset("final"), asset("object"), asset("background")]}
          hiddenKinds={nextHidden}
          onHiddenKindsChange={(next) => {
            hidden = next;
          }}
        />,
      );
    };
    await act(async () => rerender(hidden));
    let checkboxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    expect(checkboxes[1]?.checked).toBe(false);
    expect(checkboxes[0]?.checked).toBe(true);
    await act(async () => checkboxes[1]?.click());
    expect([...hidden]).toEqual([]);
    await act(async () => rerender(hidden));
    checkboxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    expect(checkboxes.every((box) => box.checked)).toBe(true);
  });

  it("renders no checkboxes when there are no assets", async () => {
    const { AssetVisibilityPanel } = await import("./AssetVisibilityPanel");
    await act(async () => {
      root.render(<AssetVisibilityPanel assets={[]} hiddenKinds={new Set()} onHiddenKindsChange={vi.fn()} />);
    });
    expect(container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")).toHaveLength(0);
  });
});
