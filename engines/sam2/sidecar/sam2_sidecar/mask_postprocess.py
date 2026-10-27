"""Per-frame mask post-processing: keep the object, drop scattered false positives.

SAM2 tracks the prompted object, but it also picks up look-alike regions: in
the P2 barber-chair clip the chair's reflection in the wooden floor came back
as extra blobs, which later vote themselves into ``object.ply`` as isolated
clumps. This module cleans a frame's mask before it is written:

1. label connected components (4- or 8-neighbour, run-length + union-find, so
   a 1080p frame costs milliseconds);
2. drop every component smaller than ``min_area_ratio`` of the frame;
3. optionally keep only the largest surviving component.

Semantics, pinned down because the corner cases matter:

* foreground is ``value > 127`` (masks are 0/255; anti-aliased edges from
  foreign tools still work);
* the area filter is *inclusive*: a component whose area equals
  ``min_area_ratio * width * height`` survives, only strictly smaller ones
  are dropped. ``min_area_ratio=0`` therefore disables the area filter;
* when nothing reaches the threshold the largest component is kept anyway —
  an all-blobs frame is far more likely to be a small/occluded object than
  the *absence* of the tracked object, and emitting an empty mask would
  silently delete a real frame. The output is then the largest blob rather
  than nothing;
* ``keep_largest`` picks the biggest qualifying component; ties go to the
  component that appears first in raster order.

The row format (``list[bytes]``, one ``width``-byte row per frame line) is
shared with :mod:`sam2_sidecar.mask_io`, so the filter drops straight into
the propagation loop.
"""

import math
from dataclasses import dataclass

import numpy as np

DEFAULT_MIN_AREA_RATIO = 0.005
DEFAULT_KEEP_LARGEST = True
DEFAULT_CONNECTIVITY = 8
VALID_CONNECTIVITY = (4, 8)

#: Pixels strictly above this value count as foreground (masks are 0/255).
FOREGROUND_THRESHOLD = 127

_CONFIG_KEYS = frozenset({"enabled", "minAreaRatio", "keepLargest", "connectivity"})


class MaskPostprocessConfigError(ValueError):
    """A ``maskPostprocess`` payload violated the protocol contract.

    Subclasses :class:`ValueError` so callers (and tests) can catch the plain
    builtin; the server turns it into an ``invalid_request`` error response.
    """


@dataclass(frozen=True)
class MaskPostprocessConfig:
    """Request-level configuration for :func:`postprocess_rows`."""

    enabled: bool = True
    min_area_ratio: float = DEFAULT_MIN_AREA_RATIO
    keep_largest: bool = DEFAULT_KEEP_LARGEST
    connectivity: int = DEFAULT_CONNECTIVITY

    @classmethod
    def default(cls) -> "MaskPostprocessConfig":
        return cls()

    @classmethod
    def from_protocol(cls, payload) -> "MaskPostprocessConfig":
        """Parse the optional ``maskPostprocess`` request field.

        ``None`` (field absent) yields the documented defaults, which keeps the
        protocol backward compatible: existing clients get filtering for free.
        Unknown keys are rejected so a snake_case typo cannot quietly fall back
        to defaults.
        """
        if payload is None:
            return cls()
        if not isinstance(payload, dict):
            raise MaskPostprocessConfigError("maskPostprocess must be an object")

        unknown = sorted(set(payload) - _CONFIG_KEYS)
        if unknown:
            raise MaskPostprocessConfigError(
                f"maskPostprocess has unknown field(s): {', '.join(unknown)}"
            )

        enabled = payload.get("enabled", True)
        if not isinstance(enabled, bool):
            raise MaskPostprocessConfigError("maskPostprocess.enabled must be a boolean")

        keep_largest = payload.get("keepLargest", DEFAULT_KEEP_LARGEST)
        if not isinstance(keep_largest, bool):
            raise MaskPostprocessConfigError("maskPostprocess.keepLargest must be a boolean")

        connectivity = payload.get("connectivity", DEFAULT_CONNECTIVITY)
        if isinstance(connectivity, bool) or not isinstance(connectivity, int):
            raise MaskPostprocessConfigError("maskPostprocess.connectivity must be 4 or 8")
        if connectivity not in VALID_CONNECTIVITY:
            raise MaskPostprocessConfigError(
                f"maskPostprocess.connectivity must be 4 or 8 (got {connectivity})"
            )

        return cls(
            enabled=enabled,
            min_area_ratio=_validate_ratio(payload.get("minAreaRatio", DEFAULT_MIN_AREA_RATIO)),
            keep_largest=keep_largest,
            connectivity=connectivity,
        )

    def to_protocol(self) -> dict:
        return {
            "enabled": self.enabled,
            "minAreaRatio": self.min_area_ratio,
            "keepLargest": self.keep_largest,
            "connectivity": self.connectivity,
        }


@dataclass(frozen=True)
class MaskPostprocessResult:
    """Filtered rows plus the numbers an operator wants to see."""

    rows: list[bytes]
    component_count: int
    kept_components: int
    dropped_components: int
    dropped_pixels: int

    @property
    def kept_pixels(self) -> int:
        return sum(row.count(255) for row in self.rows)


def _validate_ratio(value) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise MaskPostprocessConfigError(
            "maskPostprocess.minAreaRatio must be a number between 0 and 1"
        )
    ratio = float(value)
    if not math.isfinite(ratio) or ratio < 0.0 or ratio > 1.0:
        raise MaskPostprocessConfigError(
            f"maskPostprocess.minAreaRatio must be between 0 and 1 (got {ratio!r})"
        )
    return ratio


def postprocess_rows(
    rows: list[bytes],
    *,
    min_area_ratio: float = DEFAULT_MIN_AREA_RATIO,
    keep_largest: bool = DEFAULT_KEEP_LARGEST,
    connectivity: int = DEFAULT_CONNECTIVITY,
) -> MaskPostprocessResult:
    """Filter one frame's mask rows (see the module docstring for semantics)."""
    if isinstance(connectivity, bool) or connectivity not in VALID_CONNECTIVITY:
        raise ValueError(f"connectivity must be 4 or 8 (got {connectivity!r})")
    ratio = _validate_ratio(min_area_ratio)

    if not rows:
        return MaskPostprocessResult([], 0, 0, 0, 0)
    height = len(rows)
    width = len(rows[0])
    if any(len(row) != width for row in rows):
        raise ValueError("all mask rows must have the same width")

    frame = np.frombuffer(b"".join(rows), dtype=np.uint8).reshape(height, width)
    foreground = frame > FOREGROUND_THRESHOLD
    if not foreground.any():
        # Nothing to filter: hand the frame back untouched so a mask that is
        # already empty costs nothing.
        return MaskPostprocessResult(list(rows), 0, 0, 0, 0)

    areas, runs_by_root = _label_components(foreground, connectivity)
    threshold = ratio * width * height
    qualifying = [root for root, area in areas.items() if area >= threshold]
    if not qualifying:
        # Documented fallback: never erase the whole mask just because the
        # object is small in this frame.
        qualifying = [max(areas, key=lambda root: (areas[root], -root))]

    if keep_largest:
        chosen = [max(qualifying, key=lambda root: (areas[root], -root))]
    else:
        chosen = qualifying

    out = np.zeros((height, width), dtype=np.uint8)
    kept_pixels = 0
    for root in chosen:
        for y, start, end in runs_by_root[root]:
            out[y, start : end + 1] = 255
            kept_pixels += end - start + 1

    total_pixels = sum(areas.values())
    payload = out.tobytes()
    out_rows = [payload[y * width : (y + 1) * width] for y in range(height)]
    return MaskPostprocessResult(
        rows=out_rows,
        component_count=len(areas),
        kept_components=len(chosen),
        dropped_components=len(areas) - len(chosen),
        dropped_pixels=total_pixels - kept_pixels,
    )


def _row_runs(row: np.ndarray) -> tuple[list[int], list[int]]:
    """Return the (inclusive) start/end columns of each foreground run in a row."""
    padded = np.zeros(row.size + 2, dtype=np.int8)
    padded[1:-1] = row
    edges = np.diff(padded)
    starts = np.flatnonzero(edges == 1).tolist()
    ends = (np.flatnonzero(edges == -1) - 1).tolist()
    return starts, ends


def _find(parent: list[int], node: int) -> int:
    while parent[node] != node:
        parent[node] = parent[parent[node]]  # path halving
        node = parent[node]
    return node


def _union(parent: list[int], left: int, right: int) -> None:
    root_left, root_right = _find(parent, left), _find(parent, right)
    if root_left == root_right:
        return
    # Smallest run index wins, which makes "first in raster order" well defined
    # for area ties and keeps results deterministic.
    if root_left < root_right:
        parent[root_right] = root_left
    else:
        parent[root_left] = root_right


def _label_components(
    foreground: np.ndarray, connectivity: int
) -> tuple[dict[int, int], dict[int, list[tuple[int, int, int]]]]:
    """Label runs of foreground pixels and merge touching runs across rows.

    Runs are the unit of work: a 960x540 mask has a few hundred of them, not
    half a million pixels, so union-find stays cheap. For 8-connectivity runs
    in adjacent rows also merge when they merely touch diagonally (a one-pixel
    horizontal gap); for 4-connectivity they must overlap.

    Returns ``(area_by_root, runs_by_root)``.
    """
    height = foreground.shape[0]
    gap = 1 if connectivity == 8 else 0

    runs: list[tuple[int, int, int]] = []  # (row, start, end)
    parent: list[int] = []
    prev_first = prev_last = 0
    for y in range(height):
        row = foreground[y]
        starts, ends = _row_runs(row) if row.any() else ([], [])
        first = len(runs)
        for start, end in zip(starts, ends, strict=True):
            runs.append((y, start, end))
            parent.append(len(parent))

        if prev_last > prev_first and len(runs) > first:
            # Both rows' runs are sorted and disjoint, so one monotone pointer
            # finds every touching pair.
            candidate = prev_first
            for index in range(first, len(runs)):
                _, start, end = runs[index]
                while candidate < prev_last and runs[candidate][2] < start - gap:
                    candidate += 1
                other = candidate
                while other < prev_last and runs[other][1] <= end + gap:
                    _union(parent, index, other)
                    other += 1
        prev_first, prev_last = first, len(runs)

    areas: dict[int, int] = {}
    runs_by_root: dict[int, list[tuple[int, int, int]]] = {}
    for index, run in enumerate(runs):
        root = _find(parent, index)
        areas[root] = areas.get(root, 0) + (run[2] - run[1] + 1)
        runs_by_root.setdefault(root, []).append(run)
    return areas, runs_by_root
