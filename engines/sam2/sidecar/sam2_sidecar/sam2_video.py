"""SAM2 video-predictor glue (real backend only).

This module is imported lazily by :class:`sam2_sidecar.backends.Sam2Backend`
and therefore only loads on machines with the SAM2 runtime installed
(``torch``, ``torchvision``, ``sam2``). It wraps the official
``sam2.sam2_video_predictor`` API: an ``inference_state`` is created once per
session from the staged frame directory, prompts are registered on their
anchor frame, and ``propagate_in_video`` fills a per-object mask cache that
``segment_frame`` drains up to the requested frame index.
"""

import json
from pathlib import Path

import numpy as np  # torch runtime dep; sam2_video is only imported by the real backend


def _png_to_jpeg(src: Path, dst: Path) -> None:
    """Re-encode a PNG frame to JPEG for the SAM2 video loader.

    SAM2's frame loader only accepts JPEG files; decoding is done with
    torchvision (already a SAM2 runtime dependency) so no extra packages
    are needed. Quality 95 keeps mask-relevant detail intact.
    """
    import io

    from PIL import Image

    img = Image.open(src).convert("RGB")
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=95)
    dst.write_bytes(buf.getvalue())


def init_video_state(predictor, session: dict) -> dict:
    """Create a SAM2 ``inference_state`` for the session's staged frames.

    Frames are copied (in session order) into a temp directory with
    zero-padded names because the SAM2 video loader reads a directory of
    sequentially numbered images.
    """
    import tempfile

    staging_root = Path(tempfile.mkdtemp(prefix="sam2-session-"))
    frames_dir = staging_root / "sam2-input"
    frames_dir.mkdir(parents=True, exist_ok=True)
    for i, frame in enumerate(session["frames"]):
        dst = frames_dir / f"{i:07d}.jpg"
        if not dst.is_file():
            _png_to_jpeg(Path(frame["path"]), dst)

    state = predictor.init_state(
        video_path=str(frames_dir),
        offload_video_to_cpu=True,
        offload_state_to_cpu=True,
    )
    return {
        "inference_state": state,
        "staged_dir": str(frames_dir),
        "prompts": {},  # objectId -> prompt dict
        "registered": {},  # objectId -> set of anchor frame indices already registered
        "prompt_signature": {},  # objectId -> guidance signature at last propagation
        "propagated": {},  # objectId -> {frame_index: bool ndarray}
        "propagated_upto": {},  # objectId -> last frame index drained
    }


def _register_prompt(predictor, sam2_state: dict, object_id: str, prompt: dict) -> None:
    """Register point and/or box prompts on the anchor frame (once)."""
    if object_id in sam2_state["prompts"]:
        return
    state = sam2_state["inference_state"]
    frame_idx = int(prompt.get("anchorFrameIndex", 0))
    kwargs = {}
    if prompt.get("points"):
        kwargs["points"] = [list(map(float, p)) for p in prompt["points"]]
        kwargs["labels"] = [int(l) for l in prompt.get("pointLabels", [])]
    if prompt.get("box"):
        kwargs["box"] = [float(v) for v in prompt["box"]]
    predictor.add_new_points_or_box(state, obj_id=object_id, frame_idx=frame_idx, **kwargs)
    sam2_state["prompts"][object_id] = prompt


def _anchor_geometry(anchor: dict) -> dict:
    """Map a protocol anchor dict to add_new_points_or_box kwargs."""
    kwargs = {}
    if anchor.get("points"):
        kwargs["points"] = [list(map(float, p)) for p in anchor["points"]]
        kwargs["labels"] = [int(l) for l in anchor.get("pointLabels", [])]
    if anchor.get("box"):
        kwargs["box"] = [float(v) for v in anchor["box"]]
    return kwargs


def _register_anchors(
    predictor, sam2_state: dict, object_id: str, prompt: dict, frame_index: int
) -> None:
    """Register every anchor up to ``frame_index`` that is not registered yet.

    The first (base) anchor uses the prompt's top-level geometry via
    :func:`_register_prompt`; later anchors (re-prompts against loop drift)
    are registered as the incremental propagation reaches their frame.
    Registering an anchor invalidates cached masks from that frame onward so
    they are recomputed with the new guidance.
    """
    anchors = prompt.get("anchors")
    if not anchors:
        _register_prompt(predictor, sam2_state, object_id, prompt)
        return
    registered = sam2_state["registered"].setdefault(object_id, set())
    state = sam2_state["inference_state"]
    for anchor in sorted(anchors, key=lambda a: int(a.get("frameIndex", 0))):
        anchor_idx = int(anchor.get("frameIndex", 0))
        if anchor_idx in registered or anchor_idx > frame_index:
            continue
        if not anchor.get("points") and not anchor.get("box"):
            continue  # protocol validated this; defensive against stale states
        predictor.add_new_points_or_box(
            state, obj_id=object_id, frame_idx=anchor_idx, **_anchor_geometry(anchor)
        )
        registered.add(anchor_idx)
        # Masks already produced at/after this frame were computed without the
        # new prompt: drop them so propagation redoes them.
        cache = sam2_state["propagated"].get(object_id)
        if cache:
            for idx in [i for i in cache if i >= anchor_idx]:
                del cache[idx]
        upto = sam2_state["propagated_upto"].get(object_id, -1)
        if upto >= anchor_idx:
            sam2_state["propagated_upto"][object_id] = anchor_idx - 1


def _prompt_signature(prompt: dict) -> str:
    """Stable identity of the prompt's guidance (geometry + anchors)."""
    keys = ("objectId", "points", "pointLabels", "box", "anchorFrameIndex", "anchors")
    payload = {k: prompt.get(k) for k in keys if prompt.get(k) is not None}
    return _canonical(payload)


def _canonical(obj) -> str:
    return json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def segment_frame(predictor, sam2_state: dict, prompt: dict, frame_index: int):
    """Return the boolean mask for ``prompt``'s object at ``frame_index``.

    Propagation is incremental: results are cached so repeated requests for
    the same object only advance the predictor forward. When the prompt's
    guidance changes (anchors added/edited between propagate runs), cached
    masks are dropped and propagation restarts from scratch.
    """
    object_id = prompt["objectId"]
    signature = _prompt_signature(prompt)
    signatures = sam2_state.setdefault("prompt_signature", {})
    if signatures.get(object_id) != signature:
        # Guidance changed since the last propagation (re-anchor, geometry
        # edit): stale masks must not leak into the new run.
        sam2_state["prompts"].pop(object_id, None)
        sam2_state.setdefault("registered", {}).pop(object_id, None)
        sam2_state["propagated"].pop(object_id, None)
        sam2_state["propagated_upto"].pop(object_id, None)
        signatures[object_id] = signature

    _register_anchors(predictor, sam2_state, object_id, prompt, frame_index)

    cache = sam2_state["propagated"].setdefault(object_id, {})
    upto = sam2_state["propagated_upto"].get(object_id, -1)
    if frame_index > upto:
        state = sam2_state["inference_state"]
        for idx, _obj_ids, video_res_masks in predictor.propagate_in_video(
            state, start_frame_idx=upto + 1
        ):
            # SAM2 emits float32 *logits* (pre-sigmoid) at video resolution.
            # Threshold at 0 (as in the official SAM2 reference code): a
            # plain ``.bool()`` maps every nonzero logit to True, and real
            # logits are ~±10..±20 and never exactly 0, which yielded
            # full-frame foreground masks (coverage == 1.0, issue #1).
            raw = video_res_masks[0]
            if hasattr(raw, "cpu"):
                raw = raw.cpu()
            if hasattr(raw, "numpy"):
                raw = raw.numpy()
            # Store packed bits (not float masks): the full-video cache at
            # 1 bit/pixel is a few MB per clip, while float32 logits OOM long
            # clips. Packed masks also stay available after propagation ends,
            # so the per-frame drain loop can re-read any frame.
            boolean = np.asarray(raw) > 0.0
            cache[int(idx)] = (boolean.shape, np.packbits(boolean))
        sam2_state["propagated_upto"][object_id] = frame_index

    if frame_index not in cache:
        raise KeyError(f"SAM2 produced no mask for frame index {frame_index}")
    shape, packed = cache[frame_index]
    return np.unpackbits(packed)[: int(np.prod(shape))].reshape(shape) > 0
