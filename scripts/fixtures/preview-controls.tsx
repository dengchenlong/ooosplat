// Production controls, isolated from project writes and GPU/model loading.
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { FrameSamplingControl } from "../../src/components/FrameSamplingControl";
import { RoamSpeedControl } from "../../src/components/GaussianViewer/RoamSpeedControl";
import { PreviewEditFocus } from "../../src/components/GaussianViewer/PreviewEditFocus";
import { NumberField } from "../../src/components/GaussianViewer/TransformPanel";
import { readRoamSpeed, saveRoamSpeed } from "../../src/components/GaussianViewer/RoamPreferences";
import { LanguageProvider, useI18n } from "../../src/i18n";
import type { VideoInfo } from "../../src/types/pipeline";
import "../../src/styles.css";

const video = { fps: 30, duration: 60, totalFrames: 1800, width:1920, height:1080 } as VideoInfo;
localStorage.setItem("ooo-splat-language", new URLSearchParams(location.search).get("locale") === "en" ? "en" : "zh-CN");
function Fixture() {
  const { t } = useI18n();
  const [ratio, setRatio] = useState<number | null>(null);
  const [speed, setSpeed] = useState(readRoamSpeed);
  const [position, setPosition] = useState(0);
  const canvas = useRef<HTMLCanvasElement>(null);
  const focus = useRef<PreviewEditFocus | null>(null);
  useEffect(() => {
    const current = new PreviewEditFocus(() => { canvas.current?.focus({preventScroll:true}); return document.activeElement === canvas.current; }, () => true);
    focus.current = current;
    return () => current.destroy();
  }, []);
  const focusCanvas = () => focus.current?.begin()();
  return <>
    <section style={{width:480,margin:28,padding:20,background:'var(--paper)'}}>
      <FrameSamplingControl video={video} quality="balanced" value={ratio} disabled={false} onChange={setRatio} />
      <p className="field-note">{t("retry.hint")}</p>
      <NumberField label="X" name="X" value={position} onChange={setPosition} onBegin={() => {}} onCommit={() => {}} onFocusCanvas={focusCanvas} />
      <input data-check="other-input" aria-label="test field" />
    </section>
    <canvas ref={canvas} tabIndex={0} style={{margin:28,width:400,height:200,background:'#0e1117'}} />
    <footer className="preview-statusbar" style={{position:'fixed',bottom:0,left:0,right:0}}>
      <div className="preview-status-items">{Array.from({length:40},(_,i)=><span key={i}><b>STAT</b>{i} · 1,234,567</span>)}</div>
      <RoamSpeedControl value={speed} disabled={false} onChange={setSpeed} onCommit={saveRoamSpeed} onFocusCanvas={focusCanvas} />
    </footer>
  </>;
}
createRoot(document.getElementById("root")!).render(<LanguageProvider><Fixture /></LanguageProvider>);
