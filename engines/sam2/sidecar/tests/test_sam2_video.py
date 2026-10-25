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


class _RecordingPredictor:
    """Records prompt registrations; mask geometry depends on registration count."""

    def __init__(self, total_frames=4):
        self.calls = []  # list of (obj_id, frame_idx, kwargs)
        self.total_frames = total_frames

    def add_new_points_or_box(self, state, obj_id, frame_idx, **kwargs):
        self.calls.append((obj_id, frame_idx, kwargs))

    def propagate_in_video(self, state, start_frame_idx=0):
        # Before any anchor beyond the base is registered, the object sits at
        # columns 2:5; once re-registered it drifts to columns 3:6.
        col = 2 if len(self.calls) <= 1 else 3
        logits = np.full((8, 8), -6.0, dtype=np.float32)
        logits[2:5, col : col + 3] = 8.0
        for idx in range(start_frame_idx, self.total_frames):
            yield idx, ["obj"], [_LogitsStub(logits)]


def _anchors_prompt(anchors):
    return {
        "objectId": "obj",
        "anchors": [{"frameIndex": i, "points": [[4.0, 4.0]], "pointLabels": [1]} for i in anchors],
    }


class TestAnchorRegistration:
    def test_anchors_register_as_propagation_reaches_them(self):
        state = _state()
        predictor = _RecordingPredictor()
        prompt = _anchors_prompt([0, 2])
        sam2_video.segment_frame(predictor, state, prompt, 0)
        assert [c[1] for c in predictor.calls] == [0]
        sam2_video.segment_frame(predictor, state, prompt, 1)
        assert [c[1] for c in predictor.calls] == [0], "anchor 2 must wait until frame 2"
        sam2_video.segment_frame(predictor, state, prompt, 2)
        assert [c[1] for c in predictor.calls] == [0, 2]
        sam2_video.segment_frame(predictor, state, prompt, 3)
        assert [c[1] for c in predictor.calls] == [0, 2], "anchors register exactly once"

    def test_new_anchor_invalidates_cached_masks_from_its_frame(self):
        state = _state()
        predictor = _RecordingPredictor()
        prompt = _anchors_prompt([0])
        mask0 = sam2_video.segment_frame(predictor, state, prompt, 0)
        # Drift reported: user re-anchors at frame 2, masks >= 2 must recompute.
        prompt = _anchors_prompt([0, 2])
        sam2_video.segment_frame(predictor, state, prompt, 3)
        cache = state["propagated"]["obj"]
        assert 2 in cache and 3 in cache
        # The recomputed masks reflect the post-anchor geometry (columns 3:6).
        mask2 = sam2_video.decode_cached_mask(cache[2]) if hasattr(sam2_video, "decode_cached_mask") else cache[2]
        assert mask2[4, 5], "re-propagated frame must use the new guidance"
        assert not mask2[4, 2]
        assert mask0[3, 3], "frames before the anchor keep their original masks"

    def test_prompt_guidance_change_restarts_propagation(self):
        state = _state()
        predictor = _RecordingPredictor()
        sam2_video.segment_frame(predictor, state, _anchors_prompt([0]), 1)
        assert [c[1] for c in predictor.calls] == [0]
        # Same object, different guidance (anchor moved): full re-registration.
        sam2_video.segment_frame(predictor, state, _anchors_prompt([1]), 1)
        assert [c[1] for c in predictor.calls] == [0, 1]

    def test_legacy_prompt_without_anchors_still_registers(self):
        state = _state()
        predictor = _RecordingPredictor()
        prompt = {"objectId": "obj", "anchorFrameIndex": 0}
        sam2_video.segment_frame(predictor, state, prompt, 0)
        assert len(predictor.calls) == 1
        sam2_video.segment_frame(predictor, state, prompt, 1)
        assert len(predictor.calls) == 1, "base prompt must not re-register"


class _CountingPredictor(_RecordingPredictor):
    """Counts how many times the whole-clip propagation is (re)started."""

    def __init__(self, total_frames=6):
        super().__init__(total_frames=total_frames)
        self.propagate_calls = 0

    def propagate_in_video(self, state, start_frame_idx=0):
        self.propagate_calls += 1
        yield from super().propagate_in_video(state, start_frame_idx=start_frame_idx)


def test_walking_every_frame_propagates_the_clip_once():
    """Draining frames 0..N-1 must reuse the clip-wide cache.

    Regression: ``propagated_upto`` used to be set to the requested
    ``frame_index`` although ``propagate_in_video`` had already cached every
    later frame, so each frame restarted propagation from the next index —
    O(N^2) work (184 frames ran ~60h on CPU instead of ~20min).
    """
    state = _state()
    predictor = _CountingPredictor(total_frames=6)
    prompt = {"objectId": "obj"}
    for idx in range(6):
        mask = sam2_video.segment_frame(predictor, state, prompt, idx)
        assert mask.shape == (8, 8)
    assert predictor.propagate_calls == 1, "one clip-wide propagation must serve every frame"
    assert state["propagated_upto"]["obj"] == 5, "cache must record the last propagated index"
