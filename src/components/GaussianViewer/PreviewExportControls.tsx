import { useEffect, useRef, useState } from "react";
import { ChevronDown, Download, FileCode2, Film, Box } from "lucide-react";
import { useI18n } from "../../i18n";
import type { VideoOrientation } from "../../types/pipeline";
import type { ModelFormat } from "../../lib/modelExportTypes";
export function PreviewOrientationControl({ orientation, disabled, onChange }: {
    orientation: VideoOrientation;
    disabled: boolean;
    onChange: (value: VideoOrientation) => void;
}) {
    const { t } = useI18n();
    return <div className="preview-view-control preview-orientation-control" role="group" aria-label={t("export.orientation")}>
    {(["portrait", "landscape"] as const).map(value => <button key={value} type="button" className={orientation === value ? "active" : ""} aria-pressed={orientation === value} disabled={disabled} onClick={() => onChange(value)}>{t(value === "portrait" ? "export.portrait" : "export.landscape")}</button>)}
  </div>;
}
export function PreviewExportMenu({ mode, disabled, videoSupported, videoReason, onVideo, onHtml, onModel }: {
    mode: "adjust" | "preview";
    disabled: boolean;
    videoSupported: boolean;
    videoReason: string | null;
    onVideo: () => void;
    onHtml: () => void;
    onModel: (format: ModelFormat) => void;
}) {
    const { t } = useI18n();
    const [open, setOpen] = useState(false);
    const trigger = useRef<HTMLButtonElement>(null);
    useEffect(() => { setOpen(false); }, [mode, disabled]);
    const close = () => { setOpen(false); trigger.current?.focus(); };
    useEffect(() => { if (open) trigger.current?.parentElement?.querySelector<HTMLButtonElement>("#preview-export-options button:not(:disabled)")?.focus(); }, [open]);
    return <div className="preview-export-menu" onKeyDown={event => { if (event.key === "Escape") {
        event.stopPropagation();
        close();
    } else if (["ArrowDown","ArrowUp","Home","End"].includes(event.key)) {
      event.preventDefault();
      if (!open) { if (!disabled) setOpen(true); return; }
      const buttons=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("#preview-export-options button:not(:disabled)"));
      const index=buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next=event.key==="Home"?0:event.key==="End"?buttons.length-1:(index+(event.key==="ArrowUp"?-1:1)+buttons.length)%buttons.length;
      buttons[next]?.focus();
    } }}>
    <button ref={trigger} type="button" aria-haspopup="menu" aria-expanded={open} aria-controls="preview-export-options" disabled={disabled} onClick={() => setOpen(value => !value)}><Download size={14}/>{t("export.label")}<ChevronDown size={12}/></button>
    {open && <>
      <button type="button" tabIndex={-1} aria-hidden="true" className="export-menu-dismiss" aria-label={t("common.close")} onClick={close}/>
      <div id="preview-export-options" className="preview-export-options" role="menu" aria-label={t("export.label")}>
        {(["ply","compressedPly","spz","sog"] as const).map(format=><button key={format} role="menuitem" type="button" className="export-model-option" onClick={()=>{close();onModel(format);}}><Box size={14}/><span><b>{t(`export.format.${format}`)}</b><small>{t(`export.formatHint.${format}`)}</small></span></button>)}
        <p className="export-model-hint">{t("export.compactSizeHint")}</p>
        {mode === "preview" && <button role="menuitem" type="button" disabled={!videoSupported} title={videoReason ?? undefined} onClick={() => { close(); onVideo(); }}><Film size={14}/>{t("export.video")}</button>}
        <button role="menuitem" type="button" onClick={() => { close(); onHtml(); }}><FileCode2 size={14}/>{t("export.html")}</button>
      </div>
    </>}
  </div>;
}
