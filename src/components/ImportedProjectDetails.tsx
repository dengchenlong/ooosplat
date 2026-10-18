import { CircleAlert, Database, ExternalLink, FileBox } from "lucide-react";
import { useI18n } from "../i18n";
import { taskBasename } from "../app/taskPresentation";
import { displayPath } from "../lib/displayPath";
import type { ProjectTaskDetail } from "../types/pipeline";

export function ImportedProjectDetails({ detail, onRevealSource, onRelink }: { detail: ProjectTaskDetail; onRevealSource: () => void; onRelink: () => void }) {
  const { t } = useI18n();
  const imported = detail.imported;
  if (!imported) return null;
  const sourcePath = displayPath(imported.sourceModelPath);
  const colmapPath = imported.colmapRoot ? displayPath(imported.colmapRoot) : null;
  return <div className="project-configuration-detail imported-project-configuration">
    <div className="form-section">
      <label className="field-label">{t("import.model")}</label>
      <button className="path-picker compact imported-source-picker" type="button" disabled={!imported.sourceAvailable} title={sourcePath} onClick={onRevealSource}>
        <FileBox size={18} /><span><strong>{taskBasename(sourcePath)}</strong><small>{sourcePath}</small></span><ExternalLink size={15} aria-hidden="true" />
      </button>
      {!imported.sourceAvailable && <><p className="project-quality-warning"><CircleAlert size={14} />{t("import.sourceUnavailable")}</p><button className="imported-project-relink" type="button" onClick={onRelink}>{t("import.relink")}</button></>}
    </div>
    {imported.reshootEligible && colmapPath && <div className="form-section">
      <label className="field-label">{t("import.colmap")}</label>
      <div className="path-picker compact readonly" title={colmapPath}><Database size={18} /><span><strong>{taskBasename(colmapPath)}</strong><small>{colmapPath}</small></span></div>
    </div>}
  </div>;
}
