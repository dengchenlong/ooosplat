import { Entity } from "@playcanvas/react";
import { GSplat } from "@playcanvas/react/components";
import { useSplat } from "@playcanvas/react/hooks";
import { EDITABLE_SPLAT_ASSET_OPTIONS } from "./SplatLoadPolicy";
import type { ProjectAssetDescriptor } from "../../types/pipeline";

/**
 * One independent splat layer for a derived asset (object/background).
 * Each layer owns its own `useSplat` load, so large PLYs stream in
 * asynchronously and toggling visibility only mounts/unmounts the entity
 * without unloading or reloading the underlying splat asset.
 */
export function DerivedSplatLayer({ asset, assetUrl, visible, transform }: {
  asset: ProjectAssetDescriptor;
  assetUrl: string;
  visible: boolean;
  transform: { position: [number, number, number]; rotation: [number, number, number]; scale: number };
}) {
  const { asset: loaded } = useSplat(assetUrl, EDITABLE_SPLAT_ASSET_OPTIONS);
  if (!visible || !loaded) return null;
  return (
    <Entity name={`Derived Splat ${asset.kind}`} position={transform.position} rotation={transform.rotation} scale={[transform.scale, transform.scale, transform.scale]}>
      <Entity rotation={[0, 0, 0]}>
        <GSplat asset={loaded} unified />
      </Entity>
    </Entity>
  );
}
