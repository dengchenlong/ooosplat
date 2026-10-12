"""Sidecar server: reads JSON-lines requests from stdin, streams responses.

Guarantees:
- one UTF-8 JSON object per output line, flushed immediately;
- a malformed or unknown request produces an ``error`` response and the
  server keeps serving (it never dies on bad input);
- propagate checkpoints after every frame (``state.json``, written
  atomically) so an interrupted run can resume without redoing finished
  frames — resume never deadlocks because responses always stream to
  completion and the loop never blocks on input while propagating.
"""

import hashlib
import json
import sys
import traceback
from pathlib import Path

from . import (
    PROTOCOL_VERSION,
    REQ_ADD_PROMPT,
    REQ_HELLO,
    REQ_INIT_SESSION,
    REQ_PING,
    REQ_PROPAGATE,
    REQ_SHUTDOWN,
    RES_BYE,
    RES_ERROR,
    RES_HELLO,
    RES_MASK,
    RES_PONG,
    RES_PROMPT_ACCEPTED,
    RES_PROPAGATE_DONE,
    RES_SESSION_READY,
)
from .backends import BackendError
from .mask_io import coverage_of_rows, write_mask_png
from .protocol import ProtocolError, decode_message, encode_message

STATE_VERSION = 1


class RequestError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


class SidecarServer:
    def __init__(self, backend, input_stream, output_stream):
        self.backend = backend
        self.input = input_stream
        self.output = output_stream
        self.should_exit = False
        self.sessions: dict[str, dict] = {}

    # ---- loop -----------------------------------------------------------

    def serve(self) -> None:
        while not self.should_exit:
            if not self.serve_once():
                break

    def serve_once(self) -> bool:
        """Read and process one input line. Returns False on EOF."""
        line = self.input.readline()
        if line == "":
            self.should_exit = True
            return False
        if not line.strip():
            return True
        try:
            request = decode_message(line)
        except ProtocolError as exc:
            self._emit_error(exc.code, str(exc), None)
            return True
        try:
            for response in self.handle(request):
                self._emit(response)
        except RequestError as exc:
            self._emit_error(exc.code, str(exc), request.get("type"))
        except BackendError as exc:
            self._emit_error("backend_error", str(exc), request.get("type"))
        except Exception as exc:  # noqa: BLE001 - server must survive bugs
            traceback.print_exc(file=sys.stderr)
            self._emit_error("internal_error", f"{type(exc).__name__}: {exc}", request.get("type"))
        return True

    # ---- dispatch -------------------------------------------------------

    def handle(self, request: dict):
        req_type = request["type"]
        if req_type == REQ_HELLO:
            yield self._handle_hello(request)
        elif req_type == REQ_PING:
            yield {"type": RES_PONG}
        elif req_type == REQ_INIT_SESSION:
            yield self._handle_init_session(request)
        elif req_type == REQ_ADD_PROMPT:
            yield self._handle_add_prompt(request)
        elif req_type == REQ_PROPAGATE:
            yield from self._handle_propagate(request)
        elif req_type == REQ_SHUTDOWN:
            self.should_exit = True
            yield {"type": RES_BYE}
        else:  # pragma: no cover - decode_message rejects unknown types
            raise RequestError("bad_request", f"unhandled request type: {req_type}")

    def _handle_hello(self, request: dict) -> dict:
        version = request.get("protocolVersion")
        if version != PROTOCOL_VERSION:
            raise RequestError(
                "unsupported_protocol",
                f"client protocol {version!r} != supported {PROTOCOL_VERSION}",
            )
        response = {"type": RES_HELLO, "protocolVersion": PROTOCOL_VERSION}
        response.update(self.backend.describe())
        return response

    def _handle_init_session(self, request: dict) -> dict:
        session_id = _require_str(request, "sessionId")
        frames = request.get("frames")
        if not isinstance(frames, list) or not frames:
            raise RequestError("invalid_request", "frames must be a non-empty list")
        seen: set[str] = set()
        prepared = []
        for entry in frames:
            if not isinstance(entry, dict):
                raise RequestError("invalid_request", "each frame must be an object")
            frame_id = entry.get("frameId")
            path = entry.get("path")
            if not isinstance(frame_id, str) or not frame_id:
                raise RequestError("invalid_request", "frame requires a non-empty frameId")
            if frame_id in seen:
                raise RequestError("invalid_request", f"duplicate frameId: {frame_id}")
            if not isinstance(path, str) or not path:
                raise RequestError("invalid_request", f"frame {frame_id} requires a path")
            if not Path(path).is_file():
                raise RequestError("invalid_request", f"frame file not found: {path}")
            seen.add(frame_id)
            prepared.append(dict(entry))
        session = {"frames": prepared, "prompts": {}}
        for frame in prepared:
            width, height = self.backend.frame_size(frame)
            frame["width"] = width
            frame["height"] = height
        self.backend.on_init_session(session)
        self.sessions[session_id] = session
        return {
            "type": RES_SESSION_READY,
            "sessionId": session_id,
            "frameCount": len(prepared),
        }

    def _handle_add_prompt(self, request: dict) -> dict:
        session = self._require_session(request)
        object_id = _require_str(request, "objectId")
        frame_id = _require_str(request, "frameId")
        frame = _find_frame(session, frame_id)
        if frame is None:
            raise RequestError("unknown_frame", f"unknown frameId: {frame_id}")

        points = request.get("points")
        labels = request.get("pointLabels")
        box = request.get("box")
        prompt: dict = {"objectId": object_id, "anchorFrameId": frame_id}
        if points is not None or labels is not None:
            if not isinstance(points, list) or not points:
                raise RequestError("invalid_request", "points must be a non-empty list")
            if not isinstance(labels, list) or len(labels) != len(points):
                raise RequestError("invalid_request", "pointLabels must match points in length")
            for label in labels:
                if label not in (0, 1):
                    raise RequestError("invalid_request", "point labels must be 0 or 1")
            for x, y in points:
                if not isinstance(x, (int, float)) or not isinstance(y, (int, float)):
                    raise RequestError("invalid_request", "points must be [x, y] numbers")
            prompt["points"] = [[float(p[0]), float(p[1])] for p in points]
            prompt["pointLabels"] = [int(l) for l in labels]
        if box is not None:
            if (
                not isinstance(box, list)
                or len(box) != 4
                or not all(isinstance(v, (int, float)) for v in box)
            ):
                raise RequestError("invalid_request", "box must be [x0, y0, x1, y1]")
            if not (box[0] < box[2] and box[1] < box[3]):
                raise RequestError("invalid_request", "box must satisfy x0<x1 and y0<y1")
            prompt["box"] = [float(v) for v in box]
        if "points" not in prompt and "box" not in prompt:
            raise RequestError("invalid_request", "prompt requires points or box")

        frame_index = next(i for i, f in enumerate(session["frames"]) if f["frameId"] == frame_id)
        prompt["anchorFrameIndex"] = frame_index
        anchor = {"frameId": frame_id, "frameIndex": frame_index}
        for key in ("points", "pointLabels", "box"):
            if key in prompt:
                anchor[key] = prompt[key]

        existing = session["prompts"].get(object_id)
        if existing is None:
            # First prompt: the base anchor. Its geometry stays at the top
            # level so backends that only understand a single prompt keep
            # working unchanged.
            prompt["anchors"] = [anchor]
        else:
            # Re-prompt on an existing object = a new anchor (issue #2,
            # loop-drift countermeasure: one anchor per ~90° of view change).
            # A repeated prompt on an already-anchored frame replaces that
            # anchor's geometry instead of appending a duplicate.
            anchors = existing["anchors"]
            replaced = False
            for i, old in enumerate(anchors):
                if old["frameId"] == frame_id:
                    anchors[i] = anchor
                    replaced = True
                    break
            if not replaced:
                anchors.append(anchor)
            if frame_id == existing["anchorFrameId"]:
                # Correcting the base anchor: update its authoritative copy.
                for key in ("points", "pointLabels", "box"):
                    existing.pop(key, None)
                for key in ("points", "pointLabels", "box"):
                    if key in prompt:
                        existing[key] = prompt[key]
                existing["anchorFrameIndex"] = frame_index
            prompt = existing

        session["prompts"][object_id] = prompt
        return {
            "type": RES_PROMPT_ACCEPTED,
            "sessionId": request["sessionId"],
            "objectId": object_id,
            "frameId": frame_id,
            "anchorCount": len(prompt["anchors"]),
        }

    def _handle_propagate(self, request: dict):
        session = self._require_session(request)
        object_id = _require_str(request, "objectId")
        prompt = session["prompts"].get(object_id)
        if prompt is None:
            raise RequestError("unknown_object", f"no prompt for objectId: {object_id}")
        masks_dir = Path(_require_str(request, "masksDir"))
        masks_dir.mkdir(parents=True, exist_ok=True)

        frames = session["frames"]
        frames_hash = _frames_hash(session, object_id, prompt)
        done = _load_state(masks_dir, request["sessionId"], object_id, frames_hash)
        completed: dict[str, dict] = done["completed"]

        new_masks = 0
        for index, frame in enumerate(frames):
            frame_id = frame["frameId"]
            if frame_id in completed and (masks_dir / completed[frame_id]["maskPath"]).is_file():
                continue
            rows = self.backend.segment(session, prompt, frame, index)
            mask_name = f"{frame_id}.png"
            mask_path = masks_dir / mask_name
            width, height = frame["width"], frame["height"]
            write_mask_png(mask_path, width, height, rows)
            coverage = coverage_of_rows(rows)
            completed[frame_id] = {
                "frameId": frame_id,
                "maskPath": mask_name,
                "coverage": coverage,
                "width": width,
                "height": height,
            }
            new_masks += 1
            _save_state(masks_dir, request["sessionId"], object_id, frames_hash, completed)
            yield {
                "type": RES_MASK,
                "sessionId": request["sessionId"],
                "objectId": object_id,
                "frameId": frame_id,
                "maskPath": str(mask_path),
                "width": width,
                "height": height,
                "coverage": coverage,
            }
        yield {
            "type": RES_PROPAGATE_DONE,
            "sessionId": request["sessionId"],
            "objectId": object_id,
            "completed": new_masks,
            "skipped": len(completed) - new_masks,
        }

    # ---- helpers --------------------------------------------------------

    def _require_session(self, request: dict) -> dict:
        session_id = _require_str(request, "sessionId")
        session = self.sessions.get(session_id)
        if session is None:
            raise RequestError("unknown_session", f"unknown sessionId: {session_id}")
        return session

    def _emit(self, response: dict) -> None:
        self.output.write(encode_message(response))
        self.output.flush()

    def _emit_error(self, code: str, message: str, in_reply_to) -> None:
        response = {"type": RES_ERROR, "code": code, "message": message}
        if in_reply_to:
            response["inReplyTo"] = in_reply_to
        self._emit(response)


def _require_str(request: dict, key: str) -> str:
    value = request.get(key)
    if not isinstance(value, str) or not value:
        raise RequestError("invalid_request", f"{key} must be a non-empty string")
    return value


def _find_frame(session: dict, frame_id: str):
    for frame in session["frames"]:
        if frame["frameId"] == frame_id:
            return frame
    return None


def _canonical(obj) -> str:
    return json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def _frames_hash(session: dict, object_id: str, prompt: dict) -> str:
    public_prompt = {k: v for k, v in prompt.items() if not k.startswith("_")}
    payload = _canonical(
        {"frames": session["frames"], "objectId": object_id, "prompt": public_prompt}
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _state_path(masks_dir: Path) -> Path:
    return masks_dir / "state.json"


def _load_state(masks_dir: Path, session_id: str, object_id: str, frames_hash: str) -> dict:
    """Load a compatible checkpoint, or an empty one when absent/mismatched."""
    path = _state_path(masks_dir)
    if path.is_file():
        try:
            state = json.loads(path.read_text(encoding="utf-8"))
            if (
                state.get("version") == STATE_VERSION
                and state.get("sessionId") == session_id
                and state.get("objectId") == object_id
                and state.get("framesHash") == frames_hash
                and isinstance(state.get("completed"), dict)
            ):
                return state
        except (OSError, json.JSONDecodeError):
            pass  # corrupt checkpoint: start over
    return {"completed": {}}


def _save_state(
    masks_dir: Path,
    session_id: str,
    object_id: str,
    frames_hash: str,
    completed: dict,
) -> None:
    state = {
        "version": STATE_VERSION,
        "sessionId": session_id,
        "objectId": object_id,
        "framesHash": frames_hash,
        "completed": completed,
    }
    path = _state_path(masks_dir)
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(_canonical(state), encoding="utf-8")
    tmp.replace(path)


def main(argv=None) -> int:
    import argparse

    from .backends import create_backend

    parser = argparse.ArgumentParser(
        prog="sam2-sidecar",
        description="OOOSplat SAM2 sidecar (JSON-lines over stdin/stdout, local-only inference)",
    )
    parser.add_argument("--backend", choices=["synthetic", "sam2"], default="sam2")
    parser.add_argument(
        "--checkpoint",
        help="Path to SAM2 weights (required for --backend sam2)",
    )
    parser.add_argument(
        "--model-cfg",
        default="configs/sam2.1/sam2.1_hiera_t.yaml",
        help="SAM2 model config name (real backend)",
    )
    args = parser.parse_args(argv)

    try:
        backend = create_backend(args.backend, args.checkpoint, args.model_cfg)
    except BackendError as exc:
        print(f"sam2-sidecar: {exc}", file=sys.stderr)
        return 2

    try:
        sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    except AttributeError:  # pragma: no cover - non-CPython fallback
        pass
    server = SidecarServer(backend, sys.stdin, sys.stdout)
    server.serve()
    return 0
