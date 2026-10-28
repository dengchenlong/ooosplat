import { useId, type CSSProperties } from "react";
import { useI18n } from "../../i18n";
import { MAX_ROAM_SPEED, MIN_ROAM_SPEED } from "./RoamPreferences";

export function RoamSpeedControl({ value, disabled, onChange, onCommit, onFocusCanvas }: {
  value: number;
  disabled: boolean;
  onChange: (speed: number) => void;
  onCommit: (speed: number) => void;
  onFocusCanvas: () => void;
}) {
  const id = useId();
  const { t } = useI18n();
  const commit = (input: HTMLInputElement) => onCommit(Number(input.value));
  return <div className={`preview-roam-speed${disabled ? " is-disabled" : ""}`}>
    <label htmlFor={id}>{t("viewer.roamSpeed")}</label>
    <input id={id} type="range" min={MIN_ROAM_SPEED} max={MAX_ROAM_SPEED} step="0.1"
      style={{ "--roam-progress": `${(value - MIN_ROAM_SPEED) / (MAX_ROAM_SPEED - MIN_ROAM_SPEED) * 100}%` } as CSSProperties}
      value={value} disabled={disabled} aria-valuetext={`${value.toFixed(1)}×`}
      onChange={event => onChange(Number(event.currentTarget.value))}
      onPointerUp={event => { if (disabled || event.button !== 0) return; commit(event.currentTarget); onFocusCanvas(); }}
      onPointerCancel={event => commit(event.currentTarget)}
      onBlur={event => commit(event.currentTarget)}
      onKeyUp={event => { if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) commit(event.currentTarget); }}
      onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); commit(event.currentTarget); onFocusCanvas(); } }} />
    <output htmlFor={id}>{value.toFixed(1)}×</output>
  </div>;
}
