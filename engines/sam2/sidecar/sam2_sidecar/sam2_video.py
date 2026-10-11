"""SAM2 video-predictor glue (real backend only).

This module is imported lazily by :class:`sam2_sidecar.backends.Sam2Backend`
and therefore only loads on machines with the SAM2 runtime installed
(``torch``, ``torchvision``, ``sam2``). It wraps the official
``sam2.sam2_video_predictor`` API: an ``inference_state`` is created once per
session from the staged frame directory, prompts are registered on their
anchor frame, and ``propagate_in_video`` fills a per-object mask cache that
``segment_frame`` drains up to the requested frame index.
"""

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

    state = predictor.init_state(video_path=str(frames_dir), offload_video_to_cpu=True)
    return {
        "inference_state": state,
        "staged_dir": str(frames_dir),
        "prompts": {},  # objectId -> prompt dict
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


def segment_frame(predictor, sam2_state: dict, prompt: dict, frame_index: int):
    """Return the boolean mask for ``prompt``'s object at ``frame_index``.

    Propagation is incremental: results are cached so repeated requests for
    the same object only advance the predictor forward.
    """
    object_id = prompt["objectId"]
    _register_prompt(predictor, sam2_state, object_id, prompt)

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
            cache[int(idx)] = np.asarray(raw) > 0.0
        sam2_state["propagated_upto"][object_id] = frame_index

    if frame_index not in cache:
        raise KeyError(f"SAM2 produced no mask for frame index {frame_index}")
    return cache[frame_index]
