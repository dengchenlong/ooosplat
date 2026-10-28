const KEY = "ooo-splat-roam-speed-v1";
export const DEFAULT_ROAM_SPEED = 1;
export const MIN_ROAM_SPEED = 0.1;
export const MAX_ROAM_SPEED = 5;

export function validRoamSpeed(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value)
    && value >= MIN_ROAM_SPEED && value <= MAX_ROAM_SPEED
    && Math.abs(value * 10 - Math.round(value * 10)) < 1e-8;
}

export function readRoamSpeed(): number {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    return validRoamSpeed(value) ? value : DEFAULT_ROAM_SPEED;
  } catch { return DEFAULT_ROAM_SPEED; }
}

export function saveRoamSpeed(value: number): void {
  if (!validRoamSpeed(value)) return;
  try { localStorage.setItem(KEY, JSON.stringify(value)); } catch { /* Navigation remains usable without storage. */ }
}
