import { BellDot, Copy, LoaderCircle, RefreshCw, X } from "lucide-react";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useI18n } from "../i18n";
import { copyUpdateText } from "../lib/updates";
import type { UpdateCheckResult, UpdateRelease } from "../types/updates";

export function UpdateSettingsRow({ result, busy, failed, cooldownSeconds, onCheck }: {
  result: UpdateCheckResult | null;
  busy: boolean;
  failed: boolean;
  cooldownSeconds: number;
  onCheck: () => void;
}) {
  const { t } = useI18n();
  const status = busy ? t("update.noticeChecking") : failed ? t("update.noticeFailed")
    : result?.status === "platformUnavailable" ? t("update.platformUnavailable")
    : result?.status === "upToDate" ? t("update.upToDate")
    : result?.status === "updateAvailable" ? t("update.availableVersion", { version: result.release?.version ?? "" })
    : t("update.noticeHint");
  return <div className="settings-row update-settings-row">
    <div><strong>{t("update.settingsTitle")}</strong><p role="status">{status}</p></div>
    <button type="button" className="update-check-button" disabled={busy || cooldownSeconds > 0}
      onClick={onCheck} title={cooldownSeconds > 0 ? t("update.cooldown", { seconds: cooldownSeconds }) : undefined}>
      {busy ? <LoaderCircle className="spin" size={14} /> : <RefreshCw size={14} />}
      {t("update.checkAction")}
    </button>
  </div>;
}

export function UpdateNoticeDialog({ currentVersion, release, onClose }: {
  currentVersion: string;
  release: UpdateRelease;
  onClose: () => void;
}) {
  const { locale, t, formatDate } = useI18n();
  const titleId = useId();
  const dialog = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const [feedback, setFeedback] = useState<"copied" | "failed" | null>(null);
  const [copying, setCopying] = useState(false);
  const mounted = useRef(false);
  const copyInFlight = useRef(false);

  useEffect(() => {
    mounted.current = true;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.focus();
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeRef.current(); }
    };
    window.addEventListener("keydown", escape);
    return () => {
      mounted.current = false;
      window.removeEventListener("keydown", escape);
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, []);

  const copy = async (text: string) => {
    if (copyInFlight.current) return;
    copyInFlight.current = true;
    setCopying(true);
    setFeedback(null);
    try {
      await copyUpdateText(text);
      if (mounted.current) setFeedback("copied");
    } catch {
      if (mounted.current) setFeedback("failed");
    } finally {
      copyInFlight.current = false;
      if (mounted.current) setCopying(false);
    }
  };

  const trapFocus = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Tab") return;
    const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
    if (!buttons?.length) return;
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) {
      event.preventDefault(); last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === dialog.current)) {
      event.preventDefault(); first.focus();
    }
  };

  const notes = release.notes[locale].length ? release.notes[locale] : release.notes[locale === "en" ? "zh-CN" : "en"];
  return <div className="privacy-backdrop update-notice-backdrop" role="presentation">
    <section ref={dialog} className="update-notice-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} onKeyDown={trapFocus}>
      <header className="settings-heading">
        <span className="settings-symbol"><BellDot size={20} /></span>
        <div><small>OOOSplat</small><h2 id={titleId}>{t("update.found")}</h2></div>
        <button type="button" aria-label={t("common.close")} onClick={onClose}><X size={18} /></button>
      </header>
      <div className="update-notice-body">
        <dl className="update-version-summary">
          <div><dt>{t("update.currentVersion")}</dt><dd>{currentVersion}</dd></div>
          <div><dt>{t("update.latestVersion")}</dt><dd>{release.version}</dd></div>
          <div><dt>{t("update.publishedAt")}</dt><dd>{formatDate(release.publishedAt)}</dd></div>
        </dl>
        <h3>{t("update.releaseNotes")}</h3>
        {notes.length ? <ul className="update-release-notes">{notes.map((note, index) => <li key={index}>{note}</li>)}</ul> : <p>{t("update.noNotes")}</p>}
        <p className="update-download-hint">{t("update.manualDownload")}</p>
        <div className="update-download-row">
          <div><strong>GitHub</strong><p className="update-download-url">{release.downloads.github}</p></div>
          <button type="button" disabled={copying} onClick={() => void copy(release.downloads.github)}><Copy size={14} />{t("update.copyGithub")}</button>
        </div>
        {release.downloads.quark && <div className="update-download-row">
          <div><strong>{t("update.quark")}</strong><p className="update-download-url">{release.downloads.quark}</p></div>
          <button type="button" disabled={copying} onClick={() => void copy(release.downloads.quark!)}><Copy size={14} />{t("update.copyQuark")}</button>
        </div>}
        {release.downloads.quark && release.downloads.quarkCode && <div className="update-download-row">
          <div><strong>{t("update.quarkCode")}</strong><p className="update-download-url">{release.downloads.quarkCode}</p></div>
          <button type="button" disabled={copying} onClick={() => void copy(release.downloads.quarkCode!)}><Copy size={14} />{t("update.copyCode")}</button>
        </div>}
        <p className={`update-copy-feedback${feedback === "failed" ? " error" : ""}`} role="status">
          {copying ? t("update.copying") : feedback ? t(feedback === "copied" ? "update.copied" : "update.copyFailed") : ""}
        </p>
      </div>
      <footer className="update-notice-actions"><button type="button" onClick={onClose}>{t("update.ignore")}</button></footer>
    </section>
  </div>;
}
