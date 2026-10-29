import { useId, useRef } from 'react';
import { useI18n } from '../i18n';
import type { Quality, VideoInfo } from '../types/pipeline';

export function FrameSamplingControl({ video, quality, value, disabled, onChange }: {
  video: VideoInfo | null; quality: Quality; value: number | null; disabled: boolean;
  onChange: (ratio: number | null) => void;
}) {
  const { t } = useI18n();
  const id = useId();
  // A legacy custom value stays a stable compatibility detent throughout a
  // gesture. Changing the controlled value must not shift the other indices.
  const sourceKey = video ? `${quality}:${video.width}:${video.height}:${video.fps}:${video.duration}:${video.totalFrames}` : quality;
  const compatibility = useRef({ sourceKey, value });
  if (compatibility.current.sourceKey !== sourceKey) {
    compatibility.current = { sourceKey, value };
  }
  if (!video || !Number.isFinite(video.fps) || video.fps <= 0) return null;
  const defaultRatio = Math.min(1, ({ fast: 6, balanced: 8, high: 12 }[quality]) / video.fps);
  const ratio = value ?? defaultRatio;
  const options = [.001, ...Array.from({ length: 10 }, (_, i) => (i + 1) / 10), defaultRatio];
  if (compatibility.current.value != null) options.push(compatibility.current.value);
  // External restored values are never rounded or rewritten.
  if (!options.some(option => Math.abs(option - ratio) < 1e-10)) options.push(ratio);
  const levels = options.sort((a, b) => a - b).filter((r, i, list) => i === 0 || Math.abs(r - list[i - 1]) >= 1e-10);
  const index = levels.findIndex(option => Math.abs(option - ratio) < 1e-10);
  return <div className="frame-sampling-control">
    <div className="field-label-row"><label htmlFor={id}>{t('sampling.label')}</label>
      <output htmlFor={id}>{(ratio * 100).toFixed(1)}%</output>
      <button type="button" disabled={disabled || value == null} onClick={() => onChange(null)}>{t('sampling.reset')}</button></div>
    <input id={id} type="range" min="0" max={levels.length - 1} step="1" value={index} disabled={disabled}
      aria-label={t('sampling.label')} aria-valuetext={`${(ratio * 100).toFixed(1)}%`}
      onChange={event => { const selected = levels[Number(event.target.value)]; if (selected != null) onChange(Math.abs(selected - defaultRatio) < 1e-10 ? null : selected); }} />
  </div>;
}
