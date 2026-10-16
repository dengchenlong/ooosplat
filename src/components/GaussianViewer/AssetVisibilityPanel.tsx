import { useI18n, type TranslationKey } from "../../i18n";
import type { ProjectAssetDescriptor } from "../../types/pipeline";

/** Assets start fully visible; an empty hidden set means "no toggle recorded yet". */
export function defaultHiddenKinds(assets: readonly ProjectAssetDescriptor[]): Set<string> {
  return new Set<string>();
}

const assetLabelKey: Record<ProjectAssetDescriptor["kind"], TranslationKey> = {
  final: "viewer.asset.final",
  object: "viewer.asset.object",
  background: "viewer.asset.background",
};

export function AssetVisibilityPanel({ assets, hiddenKinds, onHiddenKindsChange }: {
  assets: readonly ProjectAssetDescriptor[];
  hiddenKinds: ReadonlySet<string>;
  onHiddenKindsChange: (next: Set<string>) => void;
}) {
  const { t } = useI18n();
  return (
    <aside className="asset-visibility-panel" aria-label={t("viewer.assetListAria")}>
      {assets.map((asset) => (
        <label key={asset.kind} className="asset-visibility-row">
          <input
            type="checkbox"
            data-asset-kind={asset.kind}
            checked={!hiddenKinds.has(asset.kind)}
            onChange={() => {
              const next = new Set(hiddenKinds);
              if (next.has(asset.kind)) next.delete(asset.kind);
              else next.add(asset.kind);
              onHiddenKindsChange(next);
            }}
          />
          <span>{t(assetLabelKey[asset.kind])}</span>
        </label>
      ))}
    </aside>
  );
}
