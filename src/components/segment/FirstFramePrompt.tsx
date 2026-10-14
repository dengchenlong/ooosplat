import { useCallback, useRef, useState } from "react";
import {
  emptyPrompt,
  type FramePrompt,
  type PointLabel,
  type SegmentPoint,
} from "../../lib/sam2/prompt";

/**
 * First-frame segmentation prompt UI (issue #2): click points and/or drag a
 * box on one displayed frame; the resulting {@link FramePrompt} is handed to
 * the parent, which converts it to a sidecar `add_prompt` request.
 *
 * The component is frame-agnostic: pointing it at a later frame and emitting
 * a prompt there is exactly the multi-anchor re-prompt path (one anchor per
 * ~90° of view change against loop drift).
 *
 * Interaction model:
 * - point mode (default): click = include point; Alt+click = exclude point;
 * - box mode: press-drag-release draws the object's bounding box;
 * - toolbar: mode switch, undo last point, clear selection.
 */
export interface FirstFramePromptProps {
  frameId: string;
  frameUrl: string;
  /** Native frame pixel size; click positions are scaled into this space. */
  frameWidth: number;
  frameHeight: number;
  objectId?: string;
  onChange?: (prompt: FramePrompt) => void;
}

interface DragState {
  startX: number;
  startY: number;
  curX: number;
  curY: number;
}

export function FirstFramePrompt({
  frameId,
  frameUrl,
  frameWidth,
  frameHeight,
  objectId = "object",
  onChange,
}: FirstFramePromptProps) {
  const [mode, setMode] = useState<"point" | "box">("point");
  const [points, setPoints] = useState<SegmentPoint[]>([]);
  const [box, setBox] = useState<[number, number, number, number] | null>(null);
  // Refs mirror the state so consecutive events in one React batch (fast
  // clicks) always read the latest selection, not the pending render's copy.
  const pointsRef = useRef<SegmentPoint[]>([]);
  const boxRef = useRef<[number, number, number, number] | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);

  const apply = useCallback(
    (nextPoints: SegmentPoint[], nextBox: [number, number, number, number] | null) => {
      pointsRef.current = nextPoints;
      boxRef.current = nextBox;
      setPoints(nextPoints);
      setBox(nextBox);
      onChange?.({ ...emptyPrompt(objectId, frameId), points: nextPoints, box: nextBox });
    },
    [onChange, objectId, frameId],
  );

  const toNative = (
    event: { clientX: number; clientY: number; target?: EventTarget | null; currentTarget?: EventTarget | null },
  ): [number, number] => {
    const el = (event.currentTarget ?? event.target) as HTMLElement | null;
    if (!el) return [0, 0];
    const rect = el.getBoundingClientRect();
    const scaleX = frameWidth / rect.width;
    const scaleY = frameHeight / rect.height;
    return [(event.clientX - rect.left) * scaleX, (event.clientY - rect.top) * scaleY];
  };

  const handleMouseDown = (event: React.MouseEvent) => {
    if (mode !== "box") return;
    const [x, y] = toNative(event);
    const state = { startX: x, startY: y, curX: x, curY: y };
    dragRef.current = state;
    setDrag(state);
  };

  const handleMouseMove = (event: React.MouseEvent) => {
    if (!dragRef.current) return;
    const [x, y] = toNative(event);
    const state = { ...dragRef.current, curX: x, curY: y };
    dragRef.current = state;
    setDrag(state);
  };

  const handleMouseUp = (event: React.MouseEvent) => {
    const state = dragRef.current;
    dragRef.current = null;
    setDrag(null);
    if (!state) return;
    const [x, y] = toNative(event);
    const x0 = Math.min(state.startX, x);
    const y0 = Math.min(state.startY, y);
    const x1 = Math.max(state.startX, x);
    const y1 = Math.max(state.startY, y);
    if (x1 - x0 < 1 || y1 - y0 < 1) return; // click without drag: ignore
    const nextBox: [number, number, number, number] = [x0, y0, x1, y1];
    apply([], nextBox);
  };

  const handleClick = (event: React.MouseEvent) => {
    if (mode !== "point") return;
    const [x, y] = toNative(event);
    const label: PointLabel = event.altKey ? 0 : 1;
    apply([...pointsRef.current, { x, y, label }], null);
  };

  const undoPoint = () => {
    apply(pointsRef.current.slice(0, -1), boxRef.current);
  };

  const clearPrompt = () => {
    apply([], null);
  };

  const previewBox = drag
    ? {
        left: `${(Math.min(drag.startX, drag.curX) / frameWidth) * 100}%`,
        top: `${(Math.min(drag.startY, drag.curY) / frameHeight) * 100}%`,
        width: `${(Math.abs(drag.curX - drag.startX) / frameWidth) * 100}%`,
        height: `${(Math.abs(drag.curY - drag.startY) / frameHeight) * 100}%`,
      }
    : null;

  return (
    <div className="segment-prompt">
      <div className="segment-prompt-toolbar">
        <button type="button" data-testid="mode-point" onClick={() => setMode("point")}>
          点选
        </button>
        <button type="button" data-testid="mode-box" onClick={() => setMode("box")}>
          框选
        </button>
        <button type="button" data-testid="undo-point" onClick={undoPoint}>
          撤销
        </button>
        <button type="button" data-testid="clear-prompt" onClick={clearPrompt}>
          清除
        </button>
      </div>
      <div
        data-testid="prompt-overlay"
        style={{ position: "relative", width: "100%", userSelect: "none" }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onClick={handleClick}
      >
        <img
          src={frameUrl}
          alt={`frame ${frameId}`}
          draggable={false}
          style={{ width: "100%", display: "block", pointerEvents: "none" }}
        />
        {points.map((p, index) => (
          <span
            key={index}
            style={{
              position: "absolute",
              left: `${(p.x / frameWidth) * 100}%`,
              top: `${(p.y / frameHeight) * 100}%`,
              width: 8,
              height: 8,
              marginLeft: -4,
              marginTop: -4,
              borderRadius: "50%",
              background: p.label === 1 ? "#22c55e" : "#ef4444",
              pointerEvents: "none",
            }}
          />
        ))}
        {box && (
          <span
            style={{
              position: "absolute",
              left: `${(box[0] / frameWidth) * 100}%`,
              top: `${(box[1] / frameHeight) * 100}%`,
              width: `${((box[2] - box[0]) / frameWidth) * 100}%`,
              height: `${((box[3] - box[1]) / frameHeight) * 100}%`,
              border: "2px solid #3b82f6",
              pointerEvents: "none",
            }}
          />
        )}
        {previewBox && (
          <span
            style={{
              position: "absolute",
              ...previewBox,
              border: "1px dashed #3b82f6",
              pointerEvents: "none",
            }}
          />
        )}
      </div>
    </div>
  );
}
