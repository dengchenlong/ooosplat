import { PROJECTION_ORTHOGRAPHIC, PROJECTION_PERSPECTIVE } from "playcanvas";
import type { ViewerCameraState } from "./ViewerControls";

const keyFor = (projectId: string) => `ooo-splat-preview-view-v1:${projectId}`;

function validState(value: unknown): value is ViewerCameraState {
  if (!value || typeof value !== "object") return false;
  const state = value as ViewerCameraState;
  return Array.isArray(state.target) && state.target.length === 3 && state.target.every(Number.isFinite)
    && [state.yaw, state.pitch, state.distance, state.horizontalFrameOffset, state.orthoHeight].every(Number.isFinite)
    && state.distance > 0 && state.orthoHeight > 0
    && Math.abs(state.pitch) <= 90
    && (state.projection === PROJECTION_PERSPECTIVE || state.projection === PROJECTION_ORTHOGRAPHIC)
    && (state.orthographicView === null || ["side", "front", "top"].includes(state.orthographicView))
    && (state.orthographicView === null || state.projection === PROJECTION_ORTHOGRAPHIC);
}

export function readPreviewView(projectId: string): ViewerCameraState | null {
  try {
    const record = JSON.parse(localStorage.getItem(keyFor(projectId)) ?? "null") as { version?: number; state?: unknown } | null;
    return record?.version === 1 && validState(record.state) ? record.state : null;
  } catch { return null; }
}

export function savePreviewView(projectId: string, state: ViewerCameraState) {
  if (!validState(state)) return false;
  try { localStorage.setItem(keyFor(projectId), JSON.stringify({ version: 1, state })); return true; }
  catch { return false; /* Camera interaction remains available if storage is full or disabled. */ }
}
