"""In-process tests for the sidecar server loop and request dispatch."""

import io
import json
import os

import pytest
from conftest import make_png

from sam2_sidecar.backends import SyntheticBackend
from sam2_sidecar.protocol import decode_message, encode_message
from sam2_sidecar.server import SidecarServer


class Harness:
    """Runs a SidecarServer over in-memory streams and drives it line by line."""

    def __init__(self, backend=None):
        self.backend = backend or SyntheticBackend()
        self.stdin = io.StringIO()
        self.stdout = io.StringIO()
        self.server = SidecarServer(self.backend, self.stdin, self.stdout)

    def send(self, msg):
        self.stdin.write(encode_message(msg))

    def send_raw(self, line):
        self.stdin.write(line if line.endswith("\n") else line + "\n")

    def step(self):
        """Process the next buffered input line; return list of response dicts."""
        lines = self.stdin.getvalue().splitlines(keepends=True)
        self.server.input = io.StringIO(lines[0] if lines else "")
        self.server.serve_once()
        out = self.stdout.getvalue()
        responses = []
        for line in out.splitlines():
            responses.append(decode_message(line + "\n"))
        # keep only unconsumed input lines; drop consumed responses
        self.stdin = io.StringIO("".join(lines[1:]))
        self.stdout = io.StringIO()
        self.server.input = self.stdin
        self.server.output = self.stdout
        return responses

    def request(self, msg):
        """Send one request and return all responses it produced."""
        self.send(msg)
        return self.step()


@pytest.fixture
def frame_dir(tmp_path):
    d = tmp_path / "frames"
    d.mkdir()
    for i in range(4):
        make_png(d / f"{i:04d}.png", width=64, height=48, fill=(10, 200, 30))
    return d


def frames_payload(frame_dir, count=4):
    return [{"frameId": f"{i:04d}", "path": str(frame_dir / f"{i:04d}.png")} for i in range(count)]


def init_session(h, frame_dir, session_id="s1"):
    return h.request(
        {"type": "init_session", "sessionId": session_id, "frames": frames_payload(frame_dir)}
    )


class TestHello:
    def test_hello_reports_backend_and_version(self):
        h = Harness()
        resp = h.request({"type": "hello", "protocolVersion": 1})
        assert resp == [
            {
                "type": "hello",
                "protocolVersion": 1,
                "backend": "synthetic",
                "model": "sam2.1_hiera_tiny",
                "localOnly": True,
            }
        ]

    def test_unsupported_version_is_an_error_and_server_survives(self):
        h = Harness()
        resp = h.request({"type": "hello", "protocolVersion": 99})
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "unsupported_protocol"
        assert h.request({"type": "ping"})[0]["type"] == "pong"


class TestPing:
    def test_ping_pong(self):
        h = Harness()
        assert h.request({"type": "ping"}) == [{"type": "pong"}]


class TestMalformed:
    def test_malformed_json_yields_error_and_server_continues(self):
        h = Harness()
        h.send_raw("{not json}\n")
        resp = h.step()
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "bad_request"
        assert h.request({"type": "ping"})[0]["type"] == "pong"

    def test_unknown_type_yields_error(self):
        h = Harness()
        h.send_raw('{"type": "warp_drive"}\n')
        resp = h.step()
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "bad_request"


class TestInitSession:
    def test_init_session_ready(self, frame_dir):
        h = Harness()
        resp = init_session(h, frame_dir)
        assert resp == [{"type": "session_ready", "sessionId": "s1", "frameCount": 4}]

    def test_missing_frame_file_is_an_error(self, tmp_path):
        h = Harness()
        resp = h.request(
            {
                "type": "init_session",
                "sessionId": "s1",
                "frames": [{"frameId": "0000", "path": str(tmp_path / "nope.png")}],
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "invalid_request"

    def test_duplicate_frame_ids_rejected(self, frame_dir):
        h = Harness()
        payload = frames_payload(frame_dir, 2)
        payload.append(dict(payload[0]))
        resp = h.request({"type": "init_session", "sessionId": "s1", "frames": payload})
        assert resp[0]["type"] == "error"

    def test_empty_frames_rejected(self, frame_dir):
        h = Harness()
        resp = h.request({"type": "init_session", "sessionId": "s1", "frames": []})
        assert resp[0]["type"] == "error"

    def test_reinit_replaces_session(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "init_session",
                "sessionId": "s1",
                "frames": frames_payload(frame_dir, 2),
            }
        )
        assert resp == [{"type": "session_ready", "sessionId": "s1", "frameCount": 2}]


class TestAddPrompt:
    def test_point_prompt_accepted(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        assert resp == [
            {
                "type": "prompt_accepted",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "anchorCount": 1,
            }
        ]

    def test_box_prompt_accepted(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "box": [10, 10, 40, 30],
            }
        )
        assert resp[0]["type"] == "prompt_accepted"

    def test_unknown_session_rejected(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "ghost",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[1, 1]],
                "pointLabels": [1],
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "unknown_session"

    def test_unknown_frame_rejected(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "9999",
                "points": [[1, 1]],
                "pointLabels": [1],
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "unknown_frame"

    def test_prompt_without_points_or_box_rejected(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {"type": "add_prompt", "sessionId": "s1", "objectId": "obj1", "frameId": "0000"}
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "invalid_request"

    def test_point_label_count_mismatch_rejected(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[1, 1], [2, 2]],
                "pointLabels": [1],
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "invalid_request"


class TestPropagate:
    def test_propagate_streams_one_mask_per_frame_then_done(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        masks_dir = tmp_path / "masks"
        resp = h.request(
            {
                "type": "propagate",
                "sessionId": "s1",
                "objectId": "obj1",
                "masksDir": str(masks_dir),
            }
        )
        masks = [r for r in resp if r["type"] == "mask"]
        dones = [r for r in resp if r["type"] == "propagate_done"]
        assert len(masks) == 4
        assert len(dones) == 1
        assert dones[0]["completed"] == 4
        assert dones[0]["skipped"] == 0
        for i, m in enumerate(masks):
            assert m["frameId"] == f"{i:04d}"
            assert m["sessionId"] == "s1"
            assert m["objectId"] == "obj1"
            assert os.path.isfile(m["maskPath"])
            assert 0.0 < m["coverage"] <= 1.0
            assert (m["width"], m["height"]) == (64, 48)

    def test_propagate_requires_prompt(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        assert resp[0]["type"] == "prompt_accepted"
        # A second, different object has no prompt yet.
        resp = h.request(
            {
                "type": "propagate",
                "sessionId": "s1",
                "objectId": "obj2",
                "masksDir": str(tmp_path / "masks"),
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "unknown_object"

    def test_propagate_unknown_object_uses_error(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        resp = h.request(
            {
                "type": "propagate",
                "sessionId": "s1",
                "objectId": "nope",
                "masksDir": str(tmp_path / "masks"),
            }
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "unknown_object"

    def test_checkpoint_state_written(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        masks_dir = tmp_path / "masks"
        h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        state_path = masks_dir / "state.json"
        assert state_path.is_file()
        state = json.loads(state_path.read_text(encoding="utf-8"))
        assert state["sessionId"] == "s1"
        assert state["objectId"] == "obj1"
        assert len(state["completed"]) == 4

    def test_resume_skips_completed_frames(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        masks_dir = tmp_path / "masks"
        h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        # Simulate a crash that lost the last two mask files but kept state.json.
        os.remove(masks_dir / "0002.png")
        os.remove(masks_dir / "0003.png")

        resp = h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        masks = [r for r in resp if r["type"] == "mask"]
        done = next(r for r in resp if r["type"] == "propagate_done")
        assert [m["frameId"] for m in masks] == ["0002", "0003"]
        assert done["completed"] == 2
        assert done["skipped"] == 2
        assert (masks_dir / "0000.png").is_file()
        assert (masks_dir / "0003.png").is_file()

    def test_resume_with_mismatched_frames_restarts(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        masks_dir = tmp_path / "masks"
        h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        # Different session content: state must be ignored, all frames redone.
        h.request(
            {
                "type": "init_session",
                "sessionId": "s1",
                "frames": frames_payload(frame_dir, 2),
            }
        )
        h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        resp = h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        done = next(r for r in resp if r["type"] == "propagate_done")
        assert done["completed"] == 2
        assert done["skipped"] == 0


class TestAnchors:
    """Multi-anchor re-prompt (issue #2): a second add_prompt on the same
    object is a re-anchor against loop drift, not a replacement."""

    def _add_prompt(self, h, frame_id, points):
        return h.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": frame_id,
                "points": points,
                "pointLabels": [1] * len(points),
            }
        )

    def test_second_prompt_becomes_anchor(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        self._add_prompt(h, "0000", [[32, 24]])
        resp = self._add_prompt(h, "0002", [[30, 20]])
        assert resp[0]["type"] == "prompt_accepted"
        assert resp[0]["anchorCount"] == 2
        prompt = h.server.sessions["s1"]["prompts"]["obj1"]
        assert prompt["anchorFrameId"] == "0000"
        assert prompt["anchors"][0]["frameId"] == "0000"
        assert prompt["anchors"][0]["frameIndex"] == 0
        assert prompt["anchors"][1] == {
            "frameId": "0002",
            "frameIndex": 2,
            "points": [[30.0, 20.0]],
            "pointLabels": [1],
        }

    def test_anchor_at_base_frame_replaces_base_geometry(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        self._add_prompt(h, "0000", [[32, 24]])
        self._add_prompt(h, "0002", [[30, 20]])
        resp = self._add_prompt(h, "0000", [[10, 10]])
        assert resp[0]["anchorCount"] == 2, "same frameId must replace, not append"
        prompt = h.server.sessions["s1"]["prompts"]["obj1"]
        assert prompt["points"] == [[10.0, 10.0]], "base prompt geometry must update"
        assert prompt["anchors"][0]["points"] == [[10.0, 10.0]]
        assert prompt["anchors"][1]["points"] == [[30.0, 20.0]]

    def test_anchor_addition_invalidates_checkpoint(self, frame_dir, tmp_path):
        h = Harness()
        init_session(h, frame_dir)
        self._add_prompt(h, "0000", [[32, 24]])
        masks_dir = tmp_path / "masks"
        h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        before = json.loads((masks_dir / "state.json").read_text(encoding="utf-8"))
        # User reviews frames, spots drift, re-anchors at frame 0002.
        self._add_prompt(h, "0002", [[30, 20]])
        resp = h.request(
            {"type": "propagate", "sessionId": "s1", "objectId": "obj1", "masksDir": str(masks_dir)}
        )
        done = next(r for r in resp if r["type"] == "propagate_done")
        assert done["completed"] == 4, "anchors change the outcome: all frames recompute"
        assert done["skipped"] == 0
        after = json.loads((masks_dir / "state.json").read_text(encoding="utf-8"))
        assert after["framesHash"] != before["framesHash"]

    def test_anchor_rejects_empty_geometry(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        self._add_prompt(h, "0000", [[32, 24]])
        resp = h.request(
            {"type": "add_prompt", "sessionId": "s1", "objectId": "obj1", "frameId": "0002"}
        )
        assert resp[0]["type"] == "error"
        assert resp[0]["code"] == "invalid_request"
        assert len(h.server.sessions["s1"]["prompts"]["obj1"]["anchors"]) == 1


class TestShutdown:
    def test_shutdown_replies_bye(self, frame_dir):
        h = Harness()
        init_session(h, frame_dir)
        assert h.request({"type": "shutdown"}) == [{"type": "bye"}]
        assert h.server.should_exit is True
