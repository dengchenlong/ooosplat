"""Offline CLI: post-process an existing directory of mask PNGs.

For masks that were already produced (the SAM2 run is expensive, so nobody
wants to re-propagate just to drop floor reflections)::

    python -m sam2_sidecar.postprocess_masks \
        --masks-dir G:/ooosplat-engines/p2-masks-final \
        --out-dir   G:/ooosplat-engines/p2-masks-clean

Every ``*.png`` in ``--masks-dir`` is filtered and written to ``--out-dir``
under the same name; every other file (``state.json``, logs) is copied
verbatim. When ``--out-dir/state.json`` exists, each completed frame's
``coverage`` is refreshed to describe the filtered mask — the rest of the
checkpoint structure is left untouched.

The input directory is only read: writing into it is refused, so a botched
run can never destroy the raw masks.
"""

import argparse
import json
import shutil
import sys
from pathlib import Path

from .mask_io import coverage_of_rows, read_mask_png, write_mask_png
from .mask_postprocess import (
    DEFAULT_CONNECTIVITY,
    DEFAULT_KEEP_LARGEST,
    DEFAULT_MIN_AREA_RATIO,
    postprocess_rows,
)


def main(argv=None) -> int:
    args = _parse_args(argv)

    masks_dir = Path(args.masks_dir)
    out_dir = Path(args.out_dir)
    if not masks_dir.is_dir():
        print(f"postprocess_masks: not a directory: {masks_dir}", file=sys.stderr)
        return 2
    if out_dir.exists() and out_dir.resolve() == masks_dir.resolve():
        print(
            "postprocess_masks: --out-dir must differ from --masks-dir "
            "(the input directory is read-only)",
            file=sys.stderr,
        )
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)

    pngs = sorted(p for p in masks_dir.iterdir() if p.is_file() and p.suffix.lower() == ".png")
    keep_largest = not args.no_keep_largest
    print(
        f"postprocess_masks: {len(pngs)} png(s) {masks_dir} -> {out_dir} "
        f"(minAreaRatio={args.min_area_ratio}, keepLargest={keep_largest}, "
        f"connectivity={args.connectivity})"
    )

    filtered_coverage: dict[str, float] = {}
    before_total = after_total = 0.0
    dropped_components = dropped_pixels = 0
    for path in pngs:
        width, height, rows = read_mask_png(path)
        try:
            result = postprocess_rows(
                rows,
                min_area_ratio=args.min_area_ratio,
                keep_largest=keep_largest,
                connectivity=args.connectivity,
            )
        except ValueError as exc:  # pragma: no cover - argparse validates first
            print(f"postprocess_masks: {path.name}: {exc}", file=sys.stderr)
            return 2
        write_mask_png(out_dir / path.name, width, height, result.rows)

        before = coverage_of_rows(rows)
        after = coverage_of_rows(result.rows)
        filtered_coverage[path.name] = after
        before_total += before
        after_total += after
        dropped_components += result.dropped_components
        dropped_pixels += result.dropped_pixels
        print(
            f"{path.name} coverage {before:.6f} -> {after:.6f} | "
            f"components {result.component_count} -> {result.kept_components} | "
            f"dropped {result.dropped_components} ({result.dropped_pixels} px)"
        )

    for item in sorted(masks_dir.iterdir()):
        if item.is_file() and item.suffix.lower() != ".png":
            shutil.copy2(item, out_dir / item.name)

    state_path = out_dir / "state.json"
    if state_path.is_file() and filtered_coverage:
        _update_state_coverage(state_path, filtered_coverage)

    frames = len(pngs)
    mean_before = before_total / frames if frames else 0.0
    mean_after = after_total / frames if frames else 0.0
    print(
        f"processed {frames} frames, dropped {dropped_components} components "
        f"({dropped_pixels} px) | mean coverage {mean_before:.6f} -> {mean_after:.6f}"
    )
    return 0


def _parse_args(argv):
    def ratio(value: str) -> float:
        try:
            parsed = float(value)
        except ValueError as exc:
            raise argparse.ArgumentTypeError(f"not a number: {value!r}") from exc
        if not 0.0 <= parsed <= 1.0:
            raise argparse.ArgumentTypeError(f"must be between 0 and 1 (got {parsed})")
        return parsed

    parser = argparse.ArgumentParser(
        prog="python -m sam2_sidecar.postprocess_masks",
        description=(
            "Filter existing mask PNGs: keep the largest connected component and "
            "drop components below minAreaRatio of the frame (issue #14)."
        ),
    )
    parser.add_argument("--masks-dir", required=True, help="directory of mask PNGs (read-only)")
    parser.add_argument("--out-dir", required=True, help="directory to write filtered masks to")
    parser.add_argument(
        "--min-area-ratio",
        type=ratio,
        default=DEFAULT_MIN_AREA_RATIO,
        help=f"drop components smaller than this fraction of the frame "
        f"(default: {DEFAULT_MIN_AREA_RATIO})",
    )
    parser.add_argument(
        "--no-keep-largest",
        action="store_true",
        help=f"keep every component that passes the area filter "
        f"(default: keep only the largest, {DEFAULT_KEEP_LARGEST})",
    )
    parser.add_argument(
        "--connectivity",
        type=int,
        choices=(4, 8),
        default=DEFAULT_CONNECTIVITY,
        help=f"neighbourhood for connected components (default: {DEFAULT_CONNECTIVITY})",
    )
    return parser.parse_args(argv)


def _update_state_coverage(state_path: Path, coverage_by_mask: dict[str, float]) -> None:
    """Refresh ``completed[*].coverage`` for the masks we just rewrote.

    Entries whose PNG is missing from the input keep their old value: the CLI
    must not invent numbers for frames it never processed.
    """
    try:
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"postprocess_masks: cannot read {state_path}: {exc}", file=sys.stderr)
        return
    completed = state.get("completed")
    if not isinstance(completed, dict):
        return
    updated = 0
    for frame_id, entry in completed.items():
        if not isinstance(entry, dict):
            continue
        mask_name = Path(str(entry.get("maskPath") or f"{frame_id}.png")).name
        if mask_name in coverage_by_mask:
            entry["coverage"] = coverage_by_mask[mask_name]
            updated += 1
    tmp = state_path.with_suffix(".json.tmp")
    tmp.write_text(
        json.dumps(state, sort_keys=True, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    tmp.replace(state_path)
    print(f"state.json: refreshed coverage for {updated} frame(s)")


if __name__ == "__main__":  # pragma: no cover - module entry point
    raise SystemExit(main())
