export type GizmoMode = "off" | "translate" | "rotate";
const STORAGE_KEY = "ooo-splat-gizmo-mode-v1";

export function readGizmoMode(): GizmoMode {
  try {
    const mode = localStorage.getItem(STORAGE_KEY);
    return mode === "off" || mode === "rotate" || mode === "translate" ? mode : "translate";
  } catch { return "translate"; }
}

export function saveGizmoMode(mode: GizmoMode): void {
  try { localStorage.setItem(STORAGE_KEY, mode); } catch { /* A preference write must not interrupt editing. */ }
}
