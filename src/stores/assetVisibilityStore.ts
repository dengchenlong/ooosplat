import { create } from "zustand";
import type { ProjectAssetDescriptor } from "../types/pipeline";

/**
 * Per-preview-session visibility of split assets. Hidden kinds live in a set:
 * an empty set means "every listed asset is visible", which is also the
 * default before the user toggles anything (final + object + background).
 */
interface AssetVisibilityState {
  hiddenKinds: ReadonlySet<string>;
  toggle: (kind: string) => void;
  reset: () => void;
}

export const useAssetVisibilityStore = create<AssetVisibilityState>((set) => ({
  hiddenKinds: new Set<string>(),
  toggle: (kind) =>
    set((state) => {
      const next = new Set(state.hiddenKinds);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      return { hiddenKinds: next };
    }),
  reset: () => set({ hiddenKinds: new Set<string>() }),
}));

/** Kinds that should currently be mounted in the 3D scene. */
export function visibleKinds(assets: readonly ProjectAssetDescriptor[], hiddenKinds: ReadonlySet<string>): string[] {
  return assets.map((asset) => asset.kind).filter((kind) => !hiddenKinds.has(kind));
}
