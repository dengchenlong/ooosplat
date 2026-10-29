import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { localizePipelineMessage, useI18n } from "../i18n";
import { displayPath } from "../lib/displayPath";
import type { EngineKind, EngineStatus } from "../types/pipeline";

const ENGINE_LABELS: Record<EngineKind, string> = {
  ffmpeg: "FFmpeg", ffprobe: "FFprobe", colmap: "COLMAP", brush: "Brush",
};
const OPEN_DELAY_MS = 150;
const CLOSE_DELAY_MS = 200;

/** All coordinates are viewport pixels; returned sizes are pre-transform CSS pixels. */
export function enginePanelLayout(
  anchor: Pick<DOMRect, "right" | "bottom">,
  viewport: { width: number; height: number },
  scale = 1,
): CSSProperties {
  const zoom = Number.isFinite(scale) && scale > 0 ? scale : 1;
  const width = Math.min(480, Math.max(0, viewport.width - 24) / zoom);
  const left = Math.max(12, Math.min(anchor.right - width * zoom, viewport.width - width * zoom - 12));
  return {
    width,
    right: (anchor.right - left - width * zoom) / zoom,
    maxHeight: Math.min(520, Math.max(0, viewport.height - anchor.bottom - 8 * zoom - 12) / zoom),
  };
}

export function EngineStatusPopover({ engines }: { engines: EngineStatus[] }) {
  const { locale, t } = useI18n();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const restoringFocus = useRef(false);
  const [open, setOpen] = useState(false);
  const [layout, setLayout] = useState<CSSProperties>({});
  const id = useId();
  const issueCount = engines.filter(engine => !engine.canStart).length;
  const checking = engines.length === 0;
  const summary = checking ? t("top.checkingEngines") : issueCount ? t("top.engineIssues", { count: issueCount }) : t("top.enginesReady");

  const clearTimers = useCallback(() => {
    if (openTimer.current !== null) clearTimeout(openTimer.current);
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    openTimer.current = closeTimer.current = null;
  }, []);

  const dismiss = useCallback((restoreFocus = false) => {
    clearTimers();
    if (restoreFocus && panel.current?.contains(document.activeElement)) {
      restoringFocus.current = true;
      trigger.current?.focus({ preventScroll: true });
      restoringFocus.current = false;
    }
    setOpen(false);
  }, [clearTimers]);

  useEffect(() => {
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) dismiss();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented && (panel.current || openTimer.current !== null)) {
        event.preventDefault();
        dismiss(true);
      }
    };
    const hidden = () => { if (document.hidden) dismiss(); };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      clearTimers();
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", escape);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, [clearTimers, dismiss]);

  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      if (!trigger.current) return;
      const frame = root.current?.closest(".interface-frame");
      const scale = frame ? Number.parseFloat(getComputedStyle(frame).getPropertyValue("--ui-scale")) : 1;
      const next = enginePanelLayout(trigger.current.getBoundingClientRect(), {
        width: window.innerWidth, height: window.innerHeight,
      }, scale);
      setLayout(previous => previous.width === next.width && previous.right === next.right && previous.maxHeight === next.maxHeight ? previous : next);
    };
    measure();
    window.addEventListener("resize", measure);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (trigger.current) observer?.observe(trigger.current);
    const frame = root.current?.closest(".interface-frame");
    if (frame) observer?.observe(frame);
    return () => { window.removeEventListener("resize", measure); observer?.disconnect(); };
  }, [open]);

  return <div ref={root} className="engine-diagnostics" onMouseEnter={() => {
    clearTimers();
    if (!open) openTimer.current = setTimeout(() => {
      openTimer.current = null;
      setOpen(true);
    }, OPEN_DELAY_MS);
  }} onMouseLeave={() => {
    clearTimers();
    closeTimer.current = setTimeout(() => { closeTimer.current = null; setOpen(false); }, CLOSE_DELAY_MS);
  }} onFocus={() => {
    if (!restoringFocus.current) { clearTimers(); setOpen(true); }
  }} onBlur={event => {
    if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) dismiss();
  }}>
    <button ref={trigger} id={`${id}-trigger`} className="engine-summary" type="button" aria-expanded={open} aria-controls={open ? id : undefined} onClick={() => {
      clearTimers(); setOpen(true);
    }}>
      <span className={`status-light${checking ? " checking" : issueCount ? " warning" : ""}`} aria-hidden="true" />
      <span>{summary}</span>
    </button>
    {open ? <div ref={panel} id={id} className="engine-diagnostics-panel" role="region" aria-labelledby={`${id}-trigger`} tabIndex={0} style={layout}>
      {checking ? <p className="engine-checking">{summary}</p> : engines.map(engine => <section key={engine.kind} className="engine-diagnostics-item" aria-label={ENGINE_LABELS[engine.kind]}>
        <div className="engine-item-heading">
          <strong>{ENGINE_LABELS[engine.kind]}</strong>
          <div className="engine-item-flags">
            <span>{t("top.engineExists")}<b className={engine.exists ? "" : "engine-issue"}>{t(engine.exists ? "top.yes" : "top.no")}</b></span>
            <span>{t("top.engineCanStart")}<b className={engine.canStart ? "engine-available" : "engine-issue"}>{t(engine.canStart ? "top.yes" : "top.no")}</b></span>
          </div>
        </div>
        <dl className="engine-item-path"><dt>{t("top.enginePath")}</dt><dd>{displayPath(engine.path)}</dd></dl>
        {engine.detail ? <pre className={`engine-item-detail${engine.canStart ? "" : " engine-issue"}`}>{localizePipelineMessage(locale, engine.detail)}</pre> : null}
      </section>)}
    </div> : null}
  </div>;
}
