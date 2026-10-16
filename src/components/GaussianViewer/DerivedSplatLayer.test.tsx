// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectAssetDescriptor } from "../../types/pipeline";

const asset = (kind: ProjectAssetDescriptor["kind"]): ProjectAssetDescriptor => ({
  kind,
  path: `C:/proj/assets/${kind}.ply`,
  fileSize: 1024,
  splatCount: 100,
});

const loadCalls: string[] = [];

vi.mock("@playcanvas/react/hooks", () => ({
  useSplat: vi.fn((url: string) => {
    loadCalls.push(url);
    return { asset: { id: url }, loading: false, error: null, subscribe: () => () => undefined };
  }),
}));

vi.mock("@playcanvas/react", () => ({ Entity: ({ name, children }: { name: string; children?: unknown }) => <div data-name={name}>{children}</div> }));
vi.mock("@playcanvas/react/components", () => ({ GSplat: ({ asset }: { asset: unknown }) => <div data-splat={JSON.stringify(asset)} /> }));

const transform = { position: [0, 0, 0] as [number, number, number], rotation: [0, 0, 0] as [number, number, number], scale: 1 };

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

describe("DerivedSplatLayer", () => {
  it("mounts a splat entity for a visible derived asset", async () => {
    const { DerivedSplatLayer } = await import("./DerivedSplatLayer");
    await act(async () => {
      root.render(<DerivedSplatLayer asset={asset("object")} assetUrl="asset://object.ply" visible transform={transform} />);
    });
    expect(container.querySelectorAll("[data-splat]")).toHaveLength(1);
  });

  it("renders nothing while hidden but keeps its own async splat load running", async () => {
    const { DerivedSplatLayer } = await import("./DerivedSplatLayer");
    loadCalls.length = 0;
    await act(async () => {
      root.render(<DerivedSplatLayer asset={asset("background")} assetUrl="asset://background.ply" visible={false} transform={transform} />);
    });
    expect(container.querySelectorAll("[data-splat]")).toHaveLength(0);
    expect(loadCalls).toEqual(["asset://background.ply"]);
  });

  it("hides the entity and unmounts the GPU splat when toggled off", async () => {
    const { DerivedSplatLayer } = await import("./DerivedSplatLayer");
    await act(async () => {
      root.render(<DerivedSplatLayer asset={asset("object")} assetUrl="asset://object.ply" visible transform={transform} />);
    });
    expect(container.querySelectorAll("[data-splat]")).toHaveLength(1);
    await act(async () => {
      root.render(<DerivedSplatLayer asset={asset("object")} assetUrl="asset://object.ply" visible={false} transform={transform} />);
    });
    expect(container.querySelectorAll("[data-splat]")).toHaveLength(0);
  });

  it("loads each derived asset through its own useSplat call", async () => {
    const { DerivedSplatLayer } = await import("./DerivedSplatLayer");
    loadCalls.length = 0;
    await act(async () => {
      root.render(<>
        <DerivedSplatLayer asset={asset("object")} assetUrl="asset://object.ply" visible transform={transform} />
        <DerivedSplatLayer asset={asset("background")} assetUrl="asset://background.ply" visible={false} transform={transform} />
      </>);
    });
    expect(loadCalls).toEqual(["asset://object.ply", "asset://background.ply"]);
  });
});
