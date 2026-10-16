import { beforeEach, describe, expect, it } from "vitest";
import type { ProjectAssetDescriptor } from "../types/pipeline";
import { useAssetVisibilityStore, visibleKinds } from "./assetVisibilityStore";

const asset = (kind: ProjectAssetDescriptor["kind"]): ProjectAssetDescriptor => ({
  kind,
  path: `C:/proj/${kind === "final" ? "final.ply" : `assets/${kind}.ply`}`,
  fileSize: 1024,
  splatCount: 100,
});

const assets = [asset("final"), asset("object"), asset("background")];

describe("assetVisibilityStore", () => {
  beforeEach(() => useAssetVisibilityStore.getState().reset());

  it("starts with no hidden kinds so every asset is visible", () => {
    expect([...useAssetVisibilityStore.getState().hiddenKinds]).toEqual([]);
    expect(visibleKinds(assets, useAssetVisibilityStore.getState().hiddenKinds)).toEqual(["final", "object", "background"]);
  });

  it("hides only the toggled asset and shows it again on a second toggle", () => {
    const store = useAssetVisibilityStore.getState();
    store.toggle("object");
    expect([...useAssetVisibilityStore.getState().hiddenKinds]).toEqual(["object"]);
    expect(visibleKinds(assets, useAssetVisibilityStore.getState().hiddenKinds)).toEqual(["final", "background"]);
    useAssetVisibilityStore.getState().toggle("object");
    expect([...useAssetVisibilityStore.getState().hiddenKinds]).toEqual([]);
    expect(visibleKinds(assets, useAssetVisibilityStore.getState().hiddenKinds)).toEqual(["final", "object", "background"]);
  });

  it("keeps hiding independent toggles while one asset is re-shown", () => {
    const store = useAssetVisibilityStore.getState();
    store.toggle("background");
    store.toggle("final");
    useAssetVisibilityStore.getState().toggle("background");
    expect([...useAssetVisibilityStore.getState().hiddenKinds]).toEqual(["final"]);
    expect(visibleKinds(assets, useAssetVisibilityStore.getState().hiddenKinds)).toEqual(["object", "background"]);
  });

  it("reset clears every hidden kind", () => {
    const store = useAssetVisibilityStore.getState();
    store.toggle("object");
    store.toggle("background");
    useAssetVisibilityStore.getState().reset();
    expect(useAssetVisibilityStore.getState().hiddenKinds.size).toBe(0);
  });
});
