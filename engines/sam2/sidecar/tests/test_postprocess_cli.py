"""Tests for the offline mask post-processing CLI (issue #14).

The CLI sanitises a directory of already-propagated masks in place-free
fashion: input is read-only, output goes to a new directory, and an existing
``state.json`` has its per-frame ``coverage`` values refreshed.
"""

import json
import shutil

import pytest

from sam2_sidecar.mask_io import read_mask_png, rows_from_rects, write_mask_png
from sam2_sidecar.postprocess_masks import main

WIDTH, HEIGHT = 64, 48
MAIN = (8, 8, 23, 23)  # 256 px
BLOB = (52, 4, 54, 6)  # 9 px
BIG_SECOND = (40, 30, 49, 39)  # 100 px, above the 0.5% threshold (15.36 px)


def _rows(rects):
    return rows_from_rects(rects, width=WIDTH, height=HEIGHT)


def _write_masks(directory, rect_sets):
    directory.mkdir(parents=True, exist_ok=True)
    for index, rects in enumerate(rect_sets, start=1):
        write_mask_png(directory / f"frame_{index:06d}.png", WIDTH, HEIGHT, _rows(rects))
    return directory


def _state(directory, entries):
    payload = {
        "version": 1,
        "sessionId": "s1",
        "objectId": "obj1",
        "framesHash": "deadbeef",
        "completed": entries,
    }
    (directory / "state.json").write_text(json.dumps(payload), encoding="utf-8")


def _entry(frame_id, coverage):
    return {
        "frameId": frame_id,
        "maskPath": f"{frame_id}.png",
        "coverage": coverage,
        "width": WIDTH,
        "height": HEIGHT,
    }


def _pixels(path):
    _width, _height, rows = read_mask_png(path)
    return sum(row.count(255) for row in rows)


@pytest.fixture
def masks_dir(tmp_path):
    source = tmp_path / "masks"
    _write_masks(source, [[MAIN, BLOB], [MAIN, BLOB], [MAIN, BLOB]])
    _state(
        source,
        {
            "frame_000001": _entry("frame_000001", 265 / (WIDTH * HEIGHT)),
            "frame_000002": _entry("frame_000002", 265 / (WIDTH * HEIGHT)),
            "frame_000003": _entry("frame_000003", 265 / (WIDTH * HEIGHT)),
        },
    )
    (source / "notes.txt").write_text("keep me", encoding="utf-8")
    return source


def test_filters_every_png_and_reports_coverage(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"

    code = main(["--masks-dir", str(masks_dir), "--out-dir", str(out_dir)])

    captured = capsys.readouterr().out
    assert code == 0
    assert "frame_000001.png" in captured
    assert "3 frames" in captured
    for name in ("frame_000001.png", "frame_000002.png", "frame_000003.png"):
        assert _pixels(out_dir / name) == 256, "only the main object survives"


def test_input_directory_is_not_modified(tmp_path, masks_dir, capsys):
    before = {p.name: p.read_bytes() for p in masks_dir.iterdir()}
    main(["--masks-dir", str(masks_dir), "--out-dir", str(tmp_path / "clean")])
    capsys.readouterr()
    after = {p.name: p.read_bytes() for p in masks_dir.iterdir()}
    assert after == before


def test_copies_non_png_files_and_updates_state_coverage(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"

    main(["--masks-dir", str(masks_dir), "--out-dir", str(out_dir)])
    capsys.readouterr()

    assert (out_dir / "notes.txt").read_text(encoding="utf-8") == "keep me"

    state = json.loads((out_dir / "state.json").read_text(encoding="utf-8"))
    assert state["sessionId"] == "s1"
    assert state["framesHash"] == "deadbeef"
    assert set(state["completed"]) == {"frame_000001", "frame_000002", "frame_000003"}
    filtered = 256 / (WIDTH * HEIGHT)
    for entry in state["completed"].values():
        assert entry["coverage"] == pytest.approx(filtered)
        # Structure is preserved: the other fields are untouched.
        assert entry["frameId"] == entry["maskPath"].removesuffix(".png")
        assert (entry["width"], entry["height"]) == (WIDTH, HEIGHT)


def test_keep_largest_can_be_disabled(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"

    main(["--masks-dir", str(masks_dir), "--out-dir", str(out_dir), "--no-keep-largest"])
    capsys.readouterr()

    # The 100 px component clears the area threshold, so it survives too.
    assert _pixels(out_dir / "frame_000001.png") == 256 + 100


def test_min_area_ratio_is_configurable(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"

    # 10% of 3072 px = 307 px: nothing qualifies -> the largest one is kept.
    main(
        [
            "--masks-dir",
            str(masks_dir),
            "--out-dir",
            str(out_dir),
            "--min-area-ratio",
            "0.1",
        ]
    )
    capsys.readouterr()

    assert _pixels(out_dir / "frame_000001.png") == 256


def test_refuses_to_overwrite_the_input_directory(tmp_path, masks_dir):
    with pytest.raises(SystemExit) as excinfo:
        main(["--masks-dir", str(masks_dir), "--out-dir", str(masks_dir)])
    assert excinfo.value.code == 2


def test_rejects_invalid_options(tmp_path, masks_dir):
    with pytest.raises(SystemExit) as excinfo:
        main(["--masks-dir", str(masks_dir), "--out-dir", str(tmp_path / "o"), "--connectivity", "6"])
    assert excinfo.value.code == 2
    with pytest.raises(SystemExit) as excinfo:
        main(
            [
                "--masks-dir",
                str(masks_dir),
                "--out-dir",
                str(tmp_path / "o"),
                "--min-area-ratio",
                "2.0",
            ]
        )
    assert excinfo.value.code == 2


def test_missing_input_directory_is_an_error(tmp_path, capsys):
    code = main(["--masks-dir", str(tmp_path / "nope"), "--out-dir", str(tmp_path / "out")])
    assert code == 2
    assert "not a directory" in capsys.readouterr().err


def test_empty_directory_reports_zero_frames(tmp_path, capsys):
    source = tmp_path / "empty"
    source.mkdir()
    code = main(["--masks-dir", str(source), "--out-dir", str(tmp_path / "out")])
    out = capsys.readouterr().out
    assert code == 0
    assert "0 frames" in out


def test_does_not_delete_unrelated_files_already_in_out_dir(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"
    out_dir.mkdir()
    (out_dir / "keep.txt").write_text("mine", encoding="utf-8")
    main(["--masks-dir", str(masks_dir), "--out-dir", str(out_dir)])
    capsys.readouterr()
    assert (out_dir / "keep.txt").read_text(encoding="utf-8") == "mine"


def test_state_coverage_stale_entries_are_left_alone(tmp_path, masks_dir, capsys):
    out_dir = tmp_path / "clean"
    main(["--masks-dir", str(masks_dir), "--out-dir", str(out_dir)])
    capsys.readouterr()

    # Drop one png from the output and rerun into a fresh out dir: the state
    # entry without a corresponding png keeps its original coverage.
    partial = tmp_path / "partial"
    shutil.copytree(masks_dir, partial)
    (partial / "frame_000003.png").unlink()

    out2 = tmp_path / "clean2"
    main(["--masks-dir", str(partial), "--out-dir", str(out2)])
    capsys.readouterr()
    state = json.loads((out2 / "state.json").read_text(encoding="utf-8"))
    assert state["completed"]["frame_000003"]["coverage"] == pytest.approx(
        265 / (WIDTH * HEIGHT)
    )
