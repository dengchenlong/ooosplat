import { CircleAlert, Database, FileBox, FolderSearch, LoaderCircle, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { beginGaussianImport, cancelGaussianImport, completeGaussianImport, probeGaussianImport } from "../lib/backend";
import { convertGaussianImport } from "../lib/gaussianImport";
import type { GaussianImportProbe, ProjectSummary } from "../types/pipeline";
import { localizePipelineMessage, useI18n } from "../i18n";

interface Props {
  selectionId: string;
  onClose: () => void;
  onImported: (project: ProjectSummary) => void;
}

const formatBytes = (bytes: number, locale: string) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value)} ${units[unit]}`;
};

const messageOf = (error: unknown) => error instanceof Error ? error.message : String(error);

export function GaussianImportDialog({ selectionId, onClose, onImported }: Props) {
  const { locale, t } = useI18n();
  const [probe, setProbe] = useState<GaussianImportProbe | null>(null);
  const [modelId, setModelId] = useState("");
  const [colmapId, setColmapId] = useState("");
  const [name, setName] = useState("");
  const [phase, setPhase] = useState<"analysis" | "review" | "conversion" | "validation" | "registration">("analysis");
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const importId = useRef<string | null>(null);
  const cancelConversion = useRef<null | (() => void)>(null);
  const dialog = useRef<HTMLElement>(null);
  const mounted = useRef(true);
  const busy = phase !== "review" && probe !== null;

  useEffect(() => {
    mounted.current = true;
    dialog.current?.focus();
    let disposed = false;
    void probeGaussianImport(selectionId).then(value => {
      if (disposed) return;
      setProbe(value);
      setModelId(value.defaultModelId ?? value.models[0]?.id ?? "");
      setColmapId(value.defaultColmapId ?? "");
      setName(value.suggestedName);
      setPhase("review");
      setProgress(0);
    }).catch(reason => { if (!disposed) setError(localizePipelineMessage(locale, messageOf(reason))); });
    return () => {
      disposed = true;
      mounted.current = false;
      cancelConversion.current?.();
      const activeImportId = importId.current;
      importId.current = null;
      if (activeImportId) void cancelGaussianImport(activeImportId).catch(() => undefined);
    };
  }, [locale, selectionId]);

  const model = probe?.models.find(candidate => candidate.id === modelId) ?? null;
  const colmap = probe?.colmapModels.find(candidate => candidate.id === colmapId) ?? null;
  const start = async () => {
    if (!probe || !model) return;
    setError(null);
    setPhase("analysis");
    setProgress(0.02);
    let activeImportId: string | null = null;
    try {
      const session = await beginGaussianImport({ probeId: probe.probeId, modelId, colmapId: colmapId || null, projectName: name });
      activeImportId = session.importId;
      importId.current = session.importId;
      const conversion = convertGaussianImport(session, update => {
        setPhase(update.phase);
        setProgress(update.progress);
      });
      cancelConversion.current = conversion.cancel;
      const info = await conversion.result;
      cancelConversion.current = null;
      setPhase("validation");
      setProgress(0.9);
      const project = await completeGaussianImport({ importId: session.importId, ...info });
      importId.current = null;
      setPhase("registration");
      setProgress(1);
      onImported(project);
    } catch (reason) {
      if (!mounted.current) return;
      setError(localizePipelineMessage(locale, messageOf(reason)));
      if (activeImportId) await cancelGaussianImport(activeImportId).catch(() => undefined);
      try {
        const refreshed = await probeGaussianImport(selectionId);
        setProbe(refreshed);
        setModelId(refreshed.defaultModelId ?? refreshed.models[0]?.id ?? "");
        setColmapId(refreshed.defaultColmapId ?? "");
      } catch { setProbe(null); }
      importId.current = null;
      setPhase("review");
    }
  };
  const close = async () => {
    mounted.current = false;
    cancelConversion.current?.();
    if (importId.current) await cancelGaussianImport(importId.current).catch(() => undefined);
    importId.current = null;
    onClose();
  };
  const phaseLabel = useMemo(() => ({
    analysis: t("import.phase.analysis"), conversion: t("import.phase.conversion"), validation: t("import.phase.validation"), registration: t("import.phase.registration"), review: "",
  })[phase], [phase, t]);

  return <div className="privacy-backdrop gaussian-import-backdrop" role="presentation">
    <section ref={dialog} tabIndex={-1} className="gaussian-import-dialog" role="dialog" aria-modal="true" aria-labelledby="gaussian-import-title" onKeyDown={event => { if (event.key === "Escape") void close(); }}>
      <header><div><small>{t("import.eyebrow")}</small><h2 id="gaussian-import-title">{t("import.title")}</h2></div><button type="button" aria-label={t("common.close")} onClick={() => void close()}><X size={18} /></button></header>
      {!probe && !error && <div className="import-loading"><LoaderCircle className="spin" size={22} /><span>{t("import.scanning")}</span></div>}
      {error && <p className="import-error" role="alert"><CircleAlert size={16} />{error}</p>}
      {probe && <div className="gaussian-import-body">
        <label className="import-name"><span>{t("import.projectName")}</span><input value={name} maxLength={64} disabled={busy} onChange={event => setName(event.target.value)} /></label>
        <div className="import-model-choice">
          <label><span>{t("import.model")}</span><select value={modelId} disabled={busy} onChange={event => setModelId(event.target.value)}>{probe.models.map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.name}</option>)}</select></label>
          <label><span>{t("import.colmap")}</span><select value={colmapId} disabled={busy} onChange={event => setColmapId(event.target.value)}><option value="">{t("import.noColmap")}</option>{probe.colmapModels.map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.label}</option>)}</select></label>
        </div>
        <div className="import-summary-grid">
          <article><FileBox size={18} /><small>{t("import.model")}</small><strong>{model?.format ?? "—"}</strong><span>{formatBytes(model?.fileSize ?? 0, locale)} · {model?.splatCount?.toLocaleString(locale) ?? t("import.readOnImport")}</span></article>
          <article><Database size={18} /><small>COLMAP</small><strong>{colmap ? t("import.detected") : t("import.notDetected")}</strong><span>{colmap ? `${colmap.registeredImages.toLocaleString(locale)} ${t("import.images")} · ${colmap.points3d.toLocaleString(locale)} ${t("import.points")}` : t("import.editOnly")}</span></article>
          <article className={colmap?.reshootEligible ? "eligible" : "limited"}><FolderSearch size={18} /><small>{t("import.reshoot")}</small><strong>{colmap?.reshootEligible ? t("import.available") : t("import.unavailable")}</strong><span>{colmap?.reshootEligible ? t("import.readonlySnapshot") : colmap?.reason ? localizePipelineMessage(locale, colmap.reason) : t("import.missingColmap")}</span></article>
        </div>
        {model && !model.directPly && <p className="import-storage-note">{t("import.conversionEstimate", { size: formatBytes(Math.ceil(model.fileSize * 1.5), locale) })}</p>}
        {busy && <div className="import-progress" aria-live="polite"><div><span>{phaseLabel}</span><b>{Math.round(progress * 100)}%</b></div><progress max={1} value={progress} /></div>}
      </div>}
      <footer><button type="button" onClick={() => void close()}>{t("common.cancel")}</button><button className="primary-action" type="button" disabled={!probe || !model || !name.trim() || busy} onClick={() => void start()}>{busy ? <LoaderCircle className="spin" size={15} /> : null}{t("import.confirm")}</button></footer>
    </section>
  </div>;
}
