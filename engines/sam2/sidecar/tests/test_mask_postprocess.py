"""Unit tests for per-frame mask post-processing (issue #14).

The post-processor removes the scattered false positives SAM2 picks up around
a photogenic object (in P2: the barber chair mirrored in a wooden floor). It
keeps connected components, drops anything smaller than a fraction of the
frame, and optionally keeps only the largest component.

Contract under test (also documented in ``sam2_sidecar.mask_postprocess``):

* a pixel is foreground when its value is ``> 127`` (masks are 0/255);
* a component is dropped when its area is *strictly* below
  ``minAreaRatio * width * height`` — exactly at the threshold it survives;
* when nothing reaches the threshold the largest component is kept anyway, so
  a frame never loses its object entirely;
* connectivity selects whether diagonal neighbours join (8) or not (4).
"""

import pytest

from sam2_sidecar.mask_io import (
    NotAPngError,
    png_size,
    read_mask_png,
    rows_from_rects,
    write_mask_png,
)
from sam2_sidecar.mask_postprocess import (
    DEFAULT_CONNECTIVITY,
    DEFAULT_KEEP_LARGEST,
    DEFAULT_MIN_AREA_RATIO,
    DEFAULT_NEARBY_RADIUS_RATIO,
    MaskPostprocessConfig,
    postprocess_rows,
)


def coverage(rows):
    return sum(row.count(255) for row in rows) / (len(rows) * len(rows[0]))


def foreground_pixels(rows):
    return sum(row.count(255) for row in rows)


class TestKeepLargest:
    def test_keeps_main_object_and_drops_isolated_and_edge_blobs(self):
        # 40x30 frame (1200 px). Main object 10x10 = 100 px. Three single-pixel
        # specks plus an 8 px block glued to the right edge (the kind of
        # floor-reflection fragment that lands on the frame border).
        rows = rows_from_rects(
            [
                (5, 5, 14, 14),  # main object
                (30, 2, 30, 2),  # isolated speck
                (33, 4, 33, 4),  # isolated speck
                (25, 20, 25, 20),  # isolated speck
                (36, 10, 39, 11),  # 8 px block touching the right edge
            ],
            width=40,
            height=30,
        )
        assert foreground_pixels(rows) == 100 + 3 + 8

        result = postprocess_rows(rows)

        assert foreground_pixels(result.rows) == 100
        assert coverage(result.rows) == pytest.approx(100 / 1200)
        assert result.component_count == 5
        assert result.kept_components == 1
        assert result.dropped_components == 4
        assert result.dropped_pixels == 11
        # Pixels inside the main object survive; every blob pixel is cleared.
        assert result.rows[5][5] == 255
        assert result.rows[14][14] == 255
        assert result.rows[2][30] == 0
        assert result.rows[4][33] == 0
        assert result.rows[20][25] == 0
        assert result.rows[10][36] == 0
        # Rows are still plain 0/255 bytes of the original width.
        assert len(result.rows) == 30
        assert all(len(row) == 40 for row in result.rows)
        assert set(b"".join(result.rows)) == {0, 255}


class TestAreaThreshold:
    def test_component_exactly_at_threshold_survives(self):
        # 20x20 frame = 400 px; minAreaRatio 0.01 -> threshold exactly 4 px.
        # Semantics: drop only when area < threshold, so a 4 px component
        # survives and a 3 px component does not.
        rows = rows_from_rects([(0, 0, 1, 1), (5, 5, 5, 7)], width=20, height=20)
        assert foreground_pixels(rows) == 7

        result = postprocess_rows(rows, min_area_ratio=0.01, keep_largest=False)

        assert foreground_pixels(result.rows) == 4
        assert result.component_count == 2
        assert result.kept_components == 1
        assert result.rows[0][0] == 255
        assert result.rows[7][5] == 0

    def test_just_below_threshold_is_dropped(self):
        # Same threshold (4 px): the main object survives, the 3 px speck next
        # to it does not.
        rows = rows_from_rects([(0, 0, 9, 9), (12, 3, 12, 5)], width=20, height=20)
        result = postprocess_rows(rows, min_area_ratio=0.01, keep_largest=False)
        assert foreground_pixels(result.rows) == 100
        assert result.component_count == 2
        assert result.kept_components == 1
        assert result.dropped_components == 1
        assert result.dropped_pixels == 3
        assert result.rows[3][12] == 0

    def test_zero_ratio_disables_the_area_filter(self):
        rows = rows_from_rects([(0, 0, 9, 9), (15, 15, 15, 15)], width=20, height=20)
        result = postprocess_rows(rows, min_area_ratio=0.0, keep_largest=False)
        assert foreground_pixels(result.rows) == 101
        assert result.kept_components == 2


class TestEmptyAndDegenerate:
    def test_empty_mask_stays_empty(self):
        rows = [bytes(20) for _ in range(10)]
        result = postprocess_rows(rows)
        assert result.rows == rows
        assert result.component_count == 0
        assert result.kept_components == 0
        assert result.dropped_components == 0
        assert result.dropped_pixels == 0

    def test_empty_frame_list(self):
        result = postprocess_rows([])
        assert result.rows == []
        assert result.component_count == 0

    def test_all_components_below_threshold_keeps_the_largest(self):
        # Documented behaviour for "no real object in this frame": the largest
        # component is kept even though it fails the threshold. Erasing the
        # mask would silently drop the object; keeping the biggest blob only
        # leaves a smaller false positive.
        # 40x40 = 1600 px, threshold = 8 px; components are 1, 2 and 6 px.
        rows = rows_from_rects([(0, 0, 0, 0), (5, 5, 5, 6), (9, 9, 12, 9)], width=40, height=40)
        result = postprocess_rows(rows)
        assert result.component_count == 3
        assert foreground_pixels(result.rows) == 4
        assert result.rows[9][9] == 255
        assert result.rows[5][5] == 0
        assert result.rows[0][0] == 0

    def test_single_full_frame_component_is_kept(self):
        rows = [b"\xff" * 8 for _ in range(8)]
        result = postprocess_rows(rows)
        assert result.rows == rows

    def test_row_width_mismatch_is_rejected(self):
        with pytest.raises(ValueError, match="same width"):
            postprocess_rows([b"\x00" * 8, b"\x00" * 7])


class TestConnectivity:
    def _diagonal_pair(self):
        # Two 5x5 blocks that touch only at the corner (4, 4) / (5, 5).
        return rows_from_rects([(0, 0, 4, 4), (5, 5, 9, 9)], width=20, height=20)

    def test_diagonal_touch_merges_with_8_connectivity(self):
        result = postprocess_rows(self._diagonal_pair(), connectivity=8)
        assert result.component_count == 1
        assert foreground_pixels(result.rows) == 50

    def test_diagonal_touch_splits_with_4_connectivity(self):
        # Labelling is what this test pins: under 4-connectivity the diagonal
        # neighbours are two components. nearby_radius_ratio=0 keeps the strict
        # "largest only" selection, so only one of them survives.
        result = postprocess_rows(self._diagonal_pair(), connectivity=4, nearby_radius_ratio=0.0)
        assert result.component_count == 2
        # Equal areas -> the first component in raster order wins the tie.
        assert foreground_pixels(result.rows) == 25
        assert result.rows[0][0] == 255
        assert result.rows[9][9] == 0

    def test_vertical_gap_is_two_components_under_both_rules(self):
        rows = rows_from_rects([(0, 0, 4, 4), (0, 6, 4, 9)], width=10, height=10)
        assert postprocess_rows(rows, connectivity=4).component_count == 2
        assert postprocess_rows(rows, connectivity=8).component_count == 2


class TestGrayLevels:
    def test_pixels_above_127_are_foreground(self):
        # 10x4: row 0 zeros, row 1 all 127 (background), row 2 all 128
        # (foreground), row 3 zeros. Only row 2 becomes a component.
        rows = [bytes(10), bytes([127] * 10), bytes([128] * 10), bytes(10)]
        result = postprocess_rows(rows)
        assert result.component_count == 1
        assert foreground_pixels(result.rows) == 10
        assert result.rows[2][0] == 255
        assert result.rows[1][0] == 0

    def test_mixed_gray_values_are_normalised_to_255(self):
        rows = [bytes([0] * 10), bytes([200] * 10), bytes([255] * 10), bytes(10)]
        result = postprocess_rows(rows)
        assert set(b"".join(result.rows)) == {0, 255}
        assert foreground_pixels(result.rows) == 20


class TestNearbyParts:
    """A second body that touches the main blob must survive (P2 frame 213).

    In the barber-chair clip the seat cushion separates from the backrest by a
    few shadow pixels, so "keep only the largest component" erased a real chair
    part (coverage 0.436 -> 0.241). The floor reflection it is meant to remove
    sits >40 px below the chair. Keeping components that are near the main blob
    separates the two cases without a per-object threshold.
    """

    def _two_bodies(self, part_y0, part_y1=41):
        return rows_from_rects(
            [
                (10, 10, 29, 29),  # main body: 400 px
                (10, part_y0, 19, part_y1),  # second body: 100 px
            ],
            width=100,
            height=100,
        )

    def test_second_body_within_nearby_radius_is_kept(self):
        # 100x100 frame -> default radius = round(0.02 * 100) = 2 px.
        # One empty row between the bodies = distance 2 -> kept.
        rows = self._two_bodies(31, 40)
        result = postprocess_rows(rows)
        assert result.component_count == 2
        assert result.kept_components == 2
        assert foreground_pixels(result.rows) == 500
        assert result.rows[31][10] == 255
        assert result.dropped_pixels == 0

    def test_second_body_beyond_nearby_radius_is_dropped(self):
        # Same sizes, but two empty rows below the main body (distance 3 >
        # radius 2): it is a reflection, not a chair part.
        result = postprocess_rows(self._two_bodies(32, 41))
        assert result.kept_components == 1
        assert foreground_pixels(result.rows) == 400
        assert result.rows[32][10] == 0

    def test_far_large_component_is_dropped(self):
        result = postprocess_rows(self._two_bodies(50, 59))
        assert result.kept_components == 1
        assert foreground_pixels(result.rows) == 400

    def test_chain_of_nearby_parts_is_kept(self):
        # Two parts each within reach of the previous one, but the last one is
        # far from the main body: expansion has to be iterative.
        rows = rows_from_rects(
            [(10, 10, 29, 29), (10, 31, 19, 40), (10, 42, 19, 51)],
            width=100,
            height=100,
        )
        result = postprocess_rows(rows)
        assert result.kept_components == 3
        assert foreground_pixels(result.rows) == 600

    def test_zero_nearby_radius_restores_strict_largest(self):
        # Opt-out: radius 0 means "only the largest component", the pre-#14
        # behaviour, so callers that want it can still get it.
        result = postprocess_rows(self._two_bodies(31, 40), nearby_radius_ratio=0.0)
        assert result.kept_components == 1
        assert foreground_pixels(result.rows) == 400

    def test_nearby_radius_ignored_when_keep_largest_is_false(self):
        result = postprocess_rows(
            self._two_bodies(50, 59), keep_largest=False, min_area_ratio=0.005
        )
        assert result.kept_components == 2
        assert foreground_pixels(result.rows) == 500

    @pytest.mark.parametrize("ratio", [-0.1, 1.5, "0.1", None, True])
    def test_invalid_nearby_radius_rejected(self, ratio):
        with pytest.raises(ValueError, match="nearbyRadiusRatio"):
            MaskPostprocessConfig.from_protocol({"nearbyRadiusRatio": ratio})


class TestConfig:
    def test_defaults(self):
        cfg = MaskPostprocessConfig.default()
        assert cfg.enabled is True
        assert cfg.min_area_ratio == DEFAULT_MIN_AREA_RATIO == 0.005
        assert cfg.keep_largest is DEFAULT_KEEP_LARGEST is True
        assert cfg.connectivity == DEFAULT_CONNECTIVITY == 8
        assert cfg.nearby_radius_ratio == DEFAULT_NEARBY_RADIUS_RATIO == 0.02

    def test_missing_payload_means_defaults(self):
        assert MaskPostprocessConfig.from_protocol(None) == MaskPostprocessConfig.default()
        assert MaskPostprocessConfig.from_protocol({}) == MaskPostprocessConfig.default()

    def test_explicit_values_round_trip(self):
        payload = {
            "enabled": False,
            "minAreaRatio": 0.02,
            "keepLargest": False,
            "connectivity": 4,
            "nearbyRadiusRatio": 0.01,
        }
        cfg = MaskPostprocessConfig.from_protocol(payload)
        assert cfg == MaskPostprocessConfig(
            enabled=False,
            min_area_ratio=0.02,
            keep_largest=False,
            connectivity=4,
            nearby_radius_ratio=0.01,
        )
        assert cfg.to_protocol() == payload

    @pytest.mark.parametrize("ratio", [-0.1, 1.5, "0.1", None, True])
    def test_invalid_ratio_rejected(self, ratio):
        with pytest.raises(ValueError, match="minAreaRatio"):
            MaskPostprocessConfig.from_protocol({"minAreaRatio": ratio})

    @pytest.mark.parametrize("connectivity", [0, 6, 16, "8", 8.0])
    def test_invalid_connectivity_rejected(self, connectivity):
        with pytest.raises(ValueError, match="connectivity"):
            MaskPostprocessConfig.from_protocol({"connectivity": connectivity})

    @pytest.mark.parametrize("value", ["true", 1, None])
    def test_non_bool_enabled_rejected(self, value):
        with pytest.raises(ValueError, match="enabled"):
            MaskPostprocessConfig.from_protocol({"enabled": value})

    def test_unknown_key_rejected_so_typos_do_not_pass_silently(self):
        # Clients must use the camelCase protocol names; a snake_case typo
        # silently falling back to defaults would hide misconfiguration.
        with pytest.raises(ValueError, match="min_area_ratio"):
            MaskPostprocessConfig.from_protocol({"min_area_ratio": 0.01})

    def test_non_object_payload_rejected(self):
        with pytest.raises(ValueError, match="maskPostprocess"):
            MaskPostprocessConfig.from_protocol([1, 2])


class TestPngRoundTrip:
    def test_read_back_written_mask(self, tmp_path):
        rows = rows_from_rects([(2, 1, 5, 3)], width=9, height=6)
        path = tmp_path / "mask.png"
        write_mask_png(path, 9, 6, rows)

        width, height, read_rows = read_mask_png(path)

        assert (width, height) == (9, 6)
        assert read_rows == rows
        assert png_size(path) == (9, 6)

    def test_rejects_non_png(self, tmp_path):
        path = tmp_path / "not.png"
        path.write_bytes(b"hello")
        with pytest.raises(NotAPngError):
            read_mask_png(path)

    def test_rejects_rgb_png(self, tmp_path):
        from conftest import make_png

        path = make_png(tmp_path / "rgb.png", width=4, height=4)
        with pytest.raises(NotAPngError, match="grayscale"):
            read_mask_png(path)


class TestPerformance:
    def test_960x540_frame_with_noise_is_fast(self):
        # Guard against a per-pixel Python BFS creeping back in: a realistic
        # 0.5 MP frame with a main blob and 200 specks must finish in
        # milliseconds, not seconds.
        import time

        rects = [(100, 100, 400, 400)]
        # Deterministic, non-touching specks below the main blob (a 20x10 grid).
        for i in range(200):
            x = 2 + (i % 20) * 46
            y = 450 + (i // 20) * 6
            rects.append((x, y, x, y))
        rows = rows_from_rects(rects, width=960, height=540)

        started = time.perf_counter()
        result = postprocess_rows(rows)
        elapsed = time.perf_counter() - started

        assert result.component_count == 201
        assert foreground_pixels(result.rows) == 301 * 301
        assert elapsed < 3.0, f"post-processing took {elapsed:.2f}s"
