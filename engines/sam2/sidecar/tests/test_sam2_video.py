"""Regression tests for the real-backend mask conversion (issue #1).

The SAM2 video predictor yields float32 **logit** masks (pre-sigmoid).
``tensor.bool()`` maps every nonzero logit to True — logits are ~±10..±20 in
practice and never exactly 0, so that conversion produced full-frame
foreground masks (coverage == 1.0). The correct conversion thresholds at
0.0, matching the official SAM2 reference code. These tests exercise
``sam2_video.segment_frame`` with a fake predictor so they run without
torch/SAM2 installed.
"""

import pytest

np = pytest.importorskip("numpy")

from sam2_sidecar import sam2_video


def _logits_frame(value_inside=8.0, value_outside=-6.0):
    """One [8, 8] logit frame: positive 3x3 square, negative elsewhere."""
    logits = np.full((8, 8), value_outside, dtype=np.float32)
    logits[2:5, 2:5] = value_inside
    return logits


class _LogitsStub:
    """Mimics the torch-tensor API surface segment_frame relies on."""

    def __init__(self, logits):
        self._logits = np.asarray(logits, dtype=np.float32)

    def cpu(self):
        return self

    def numpy(self):
        return self._logits

    def bool(self):
        # torch semantics: any nonzero float -> True (the bug under test)
        return _BoolStub(self._logits != 0)


class _BoolStub:
    def __init__(self, arr):
        self._arr = arr

    def cpu(self):
        return self

    def numpy(self):
        return self._arr


class _FakePredictor:
    """Two-frame video: yields the same logit frame regardless of index."""

    def add_new_points_or_box(self, *args, **kwargs):
        pass

    def propagate_in_video(self, state, start_frame_idx=0):
        logits = _logits_frame()
        for idx in range(start_frame_idx, 2):
            yield idx, ["obj"], [_LogitsStub(logits)]


def _state():
    return {"prompts": {}, "propagated": {}, "propagated_upto": {}, "inference_state": object()}


def test_logits_thresholded_at_zero_not_bool():
    """Negative-logit background must stay False; only logits > 0 are mask."""
    state = _state()
    mask = sam2_video.segment_frame(_FakePredictor(), state, {"objectId": "obj"}, 0)
    assert mask.dtype == np.bool_
    assert mask.shape == (8, 8)
    assert int(mask.sum()) == 9, "expected exactly the 3x3 positive-logit square"
    assert mask[3, 3], "positive logit (inside object) must be True"
    assert not mask[0, 0], "negative logit (background) must be False"


def test_incremental_propagate_uses_cached_and_new_frames():
    state = _state()
    predictor = _FakePredictor()
    prompt = {"objectId": "obj"}
    sam2_video.segment_frame(predictor, state, prompt, 0)
    mask1 = sam2_video.segment_frame(predictor, state, prompt, 1)
    assert mask1.shape == (8, 8)
    assert mask1[3, 3] and not mask1[0, 0]
