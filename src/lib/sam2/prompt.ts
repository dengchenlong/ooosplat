/**
 * Frame prompt model shared by the first-frame selection UI and the sidecar
 * client. Coordinates are in the frame's native pixel space (the UI scales
 * click positions by the displayed/natural size ratio before storing them).
 */
import type { AddPromptRequest } from "./protocol";

export type PointLabel = 0 | 1;

export interface SegmentPoint {
  x: number;
  y: number;
  /** 1 = include (object), 0 = exclude (background). */
  label: PointLabel;
}

/** One anchor's selection on one frame. */
export interface FramePrompt {
  objectId: string;
  anchorFrameId: string;
  points: SegmentPoint[];
  /** [x0, y0, x1, y1] in native frame pixels; null when unused. */
  box: [number, number, number, number] | null;
}

export function emptyPrompt(objectId: string, anchorFrameId: string): FramePrompt {
  return { objectId, anchorFrameId, points: [], box: null };
}

/** Validation errors; an empty list means the prompt is ready for the sidecar. */
export function validateFramePrompt(prompt: FramePrompt): string[] {
  const errors: string[] = [];
  const hasBox = prompt.box !== null;
  if (prompt.points.length === 0 && !hasBox) {
    errors.push("prompt requires at least one point or a box");
  }
  if (hasBox) {
    const [x0, y0, x1, y1] = prompt.box as [number, number, number, number];
    if (!(x0 < x1 && y0 < y1)) {
      errors.push("box must satisfy x0<x1 and y0<y1");
    }
  }
  return errors;
}

export function promptHasSelection(prompt: FramePrompt): boolean {
  return validateFramePrompt(prompt).length === 0;
}

/** Convert to the sidecar `add_prompt` request. Throws on an empty selection. */
export function toAddPromptRequest(prompt: FramePrompt, sessionId: string): AddPromptRequest {
  if (!promptHasSelection(prompt)) {
    throw new Error(`prompt for object ${prompt.objectId} has no points or box`);
  }
  const request: AddPromptRequest = {
    type: "add_prompt",
    sessionId,
    objectId: prompt.objectId,
    frameId: prompt.anchorFrameId,
  };
  if (prompt.points.length > 0) {
    request.points = prompt.points.map((p) => [p.x, p.y] as [number, number]);
    request.pointLabels = prompt.points.map((p) => p.label);
  }
  if (prompt.box !== null) {
    request.box = prompt.box;
  }
  return request;
}
