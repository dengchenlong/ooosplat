"""Segmentation backends for the SAM2 sidecar.

``SyntheticBackend`` is a deterministic, dependency-free stand-in used by the
protocol and smoke tests. ``Sam2Backend`` hosts the real SAM 2.1 video
predictor locally (no cloud APIs); it requires a Python environment with
``torch`` and the ``sam2`` package plus locally distributed model weights
(see ``engines/sam2/README.md``). The real backend path needs on-device
validation on a CUDA machine and is not exercised by the protocol tests.
"""

from pathlib import Path

from .mask_io import NotAPngError, png_size, rows_from_rects


class BackendError(RuntimeError):
    """Raised when a backend cannot fulfil a request."""


class SyntheticBackend:
    """Deterministic fake segmenter: prompt shapes grown slightly per frame.

    - box prompt  -> the box itself on every frame.
    - point prompt-> a square centred on the point, base radius 12 and +2 px
      per frame index (clamped to the frame), simulating cross-frame drift.
    """

    name = "synthetic"
    model = "sam2.1_hiera_tiny"

    def describe(self) -> dict:
        return {"backend": self.name, "model": self.model, "localOnly": True}

    def on_init_session(self, session: dict) -> None:
        """Hook called after a session's frames are validated."""

    def frame_size(self, frame: dict) -> tuple[int, int]:
        width = frame.get("width")
        height = frame.get("height")
        if isinstance(width, int) and isinstance(height, int) and width > 0 and height > 0:
            return width, height
        try:
            return png_size(frame["path"])
        except (NotAPngError, OSError) as exc:
            raise BackendError(
                f"cannot determine frame size for {frame['frameId']}: {exc}"
            ) from exc

    def segment(self, session: dict, prompt: dict, frame: dict, frame_index: int) -> list[bytes]:
        """Return 0/255 row bytes for one frame."""
        width, height = frame["width"], frame["height"]
        rects = []
        if prompt.get("box"):
            x0, y0, x1, y1 = prompt["box"]
            rects.append(_clamp_rect(x0, y0, x1, y1, width, height))
        grow = 2 * frame_index
        for x, y in prompt.get("points", []):
            radius = 12 + grow
            rects.append(_clamp_rect(x - radius, y - radius, x + radius, y + radius, width, height))
        if not rects:
            raise BackendError("prompt has no points or box")
        return rows_from_rects(rects, width, height)


class Sam2Backend:
    """Real SAM 2.1 video-predictor backend (local inference only).

    Requires the sidecar environment to provide ``torch`` (CUDA build for GPU)
    and the ``sam2`` package, plus the pinned weights under the engine install
    root. All heavy imports happen lazily so machines without the runtime can
    still serve protocol traffic (and tests use ``SyntheticBackend``).
    """

    name = "sam2"

    def __init__(self, checkpoint: str, model_cfg: str):
        self.checkpoint = str(checkpoint)
        self.model_cfg = model_cfg
        self._predictor = None

    def describe(self) -> dict:
        return {
            "backend": self.name,
            "model": Path(self.model_cfg).stem,
            "localOnly": True,
            "checkpoint": self.checkpoint,
        }

    def _ensure_loaded(self):
        if self._predictor is not None:
            return
        if not Path(self.checkpoint).is_file():
            raise BackendError(
                f"SAM2 weights not found at {self.checkpoint}; "
                "install the engine first (engines/sam2/README.md)."
            )
        try:
            from sam2.build_sam import build_sam2_video_predictor
        except ImportError as exc:
            raise BackendError(
                "SAM2 runtime (torch + sam2 package) is not installed in this "
                "Python environment; see engines/sam2/README.md."
            ) from exc
        self._predictor = build_sam2_video_predictor(self.model_cfg, self.checkpoint, device="cpu")

    def on_init_session(self, session: dict) -> None:
        """Stage frames into a sequentially numbered directory and start SAM2."""
        self._ensure_loaded()
        from .sam2_video import init_video_state

        session["_sam2"] = init_video_state(self._predictor, session)

    def frame_size(self, frame: dict) -> tuple[int, int]:
        width = frame.get("width")
        height = frame.get("height")
        if isinstance(width, int) and isinstance(height, int) and width > 0 and height > 0:
            return width, height
        try:
            return png_size(frame["path"])
        except (NotAPngError, OSError) as exc:
            raise BackendError(
                f"cannot determine frame size for {frame['frameId']}: {exc}"
            ) from exc

    def segment(self, session: dict, prompt: dict, frame: dict, frame_index: int) -> list[bytes]:
        self._ensure_loaded()
        from .sam2_video import segment_frame

        mask = segment_frame(self._predictor, session["_sam2"], prompt, frame_index)
        if mask.ndim == 3:  # [1, H, W] from the video predictor
            mask = mask[0]
        return [bytes(255 if v else 0 for v in row) for row in mask.tolist()]


def _clamp_rect(x0, y0, x1, y1, width: int, height: int) -> tuple[int, int, int, int]:
    return (
        max(0, int(x0)),
        max(0, int(y0)),
        min(width - 1, int(x1)),
        min(height - 1, int(y1)),
    )


def create_backend(backend: str, checkpoint: str | None, model_cfg: str | None):
    """Build the backend requested by CLI args (``--backend synthetic|sam2``)."""
    if backend == "synthetic":
        return SyntheticBackend()
    if backend == "sam2":
        if not checkpoint:
            raise BackendError("--checkpoint is required for the sam2 backend")
        return Sam2Backend(checkpoint, model_cfg or "configs/sam2.1/sam2.1_hiera_t.yaml")
    raise BackendError(f"unknown backend: {backend}")
