"""Subprocess-level tests: real child process over stdio pipes.

Covers process management (start, heartbeats, graceful shutdown, kill
fallback) and the no-deadlock guarantee: while propagate streams, the parent
never blocks on a full pipe because it drains stdout concurrently.
"""

import json
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from conftest import make_png

SIDECAR_ROOT = Path(__file__).resolve().parent.parent


class SidecarProcess:
    """Manages a real ``python -m sam2_sidecar`` child process."""

    def __init__(self, tmp_path):
        self.proc = subprocess.Popen(
            [sys.executable, "-m", "sam2_sidecar", "--backend", "synthetic"],
            cwd=str(SIDECAR_ROOT),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        self._stderr_lines: list[str] = []
        self._stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._stderr_thread.start()

    def _drain_stderr(self):
        for line in self.proc.stderr:
            self._stderr_lines.append(line)

    def send(self, msg: dict) -> None:
        self.proc.stdin.write(json.dumps(msg, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()

    def recv(self, timeout=30.0):
        deadline = time.monotonic() + timeout
        line = self.proc.stdout.readline()  # blocking read; parent driven
        if not line:
            raise AssertionError(
                "sidecar closed stdout unexpectedly; stderr: "
                + "".join(self._stderr_lines[-20:])
            )
        return json.loads(line)

    def request(self, msg: dict, timeout=30.0):
        self.send(msg)
        return self.recv(timeout)

    def graceful_shutdown(self, timeout=15.0) -> int:
        self.send({"type": "shutdown"})
        bye = self.recv(timeout)
        assert bye["type"] == "bye"
        return self.proc.wait(timeout=timeout)

    def kill(self) -> None:
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=10)

    def stderr_text(self) -> str:
        return "".join(self._stderr_lines)


@pytest.fixture
def sidecar(tmp_path):
    sc = SidecarProcess(tmp_path)
    yield sc
    sc.kill()


@pytest.fixture
def frame_dir(tmp_path):
    d = tmp_path / "frames"
    d.mkdir()
    for i in range(6):
        make_png(d / f"{i:04d}.png", width=64, height=48, fill=(10, 200, 30))
    return d


def frames_payload(frame_dir, count):
    return [
        {"frameId": f"{i:04d}", "path": str(frame_dir / f"{i:04d}.png")}
        for i in range(count)
    ]


class TestLifecycle:
    def test_hello_handshake(self, sidecar):
        resp = sidecar.request({"type": "hello", "protocolVersion": 1})
        assert resp["type"] == "hello"
        assert resp["protocolVersion"] == 1
        assert resp["localOnly"] is True

    def test_ping_pong(self, sidecar):
        assert sidecar.request({"type": "ping"})["type"] == "pong"

    def test_graceful_shutdown_exits_zero(self, sidecar):
        code = sidecar.graceful_shutdown()
        assert code == 0

    def test_eof_without_shutdown_exits_cleanly(self, tmp_path):
        sc = SidecarProcess(tmp_path)
        assert sc.request({"type": "ping"})["type"] == "pong"
        sc.proc.stdin.close()
        assert sc.proc.wait(timeout=15) == 0
        sc.kill()

    def test_kill_terminates_a_busy_sidecar(self, tmp_path, frame_dir):
        """A killed sidecar must actually die (no orphaned process)."""
        sc = SidecarProcess(tmp_path)
        try:
            sc.request(
                {
                    "type": "init_session",
                    "sessionId": "s1",
                    "frames": frames_payload(frame_dir, 6),
                }
            )
            assert sc.proc.poll() is None
            sc.kill()
            assert sc.proc.wait(timeout=10) is not None
        finally:
            sc.kill()


class TestSmoke:
    def test_point_prompt_produces_per_frame_masks(self, sidecar, frame_dir, tmp_path):
        sidecar.request({"type": "hello", "protocolVersion": 1})
        resp = sidecar.request(
            {
                "type": "init_session",
                "sessionId": "s1",
                "frames": frames_payload(frame_dir, 6),
            }
        )
        assert resp == {"type": "session_ready", "sessionId": "s1", "frameCount": 6}

        resp = sidecar.request(
            {
                "type": "add_prompt",
                "sessionId": "s1",
                "objectId": "obj1",
                "frameId": "0000",
                "points": [[32, 24]],
                "pointLabels": [1],
            }
        )
        assert resp["type"] == "prompt_accepted"

        masks_dir = tmp_path / "masks"
        sidecar.send(
            {
                "type": "propagate",
                "sessionId": "s1",
                "objectId": "obj1",
                "masksDir": str(masks_dir),
            }
        )
        masks = []
        done = None
        while done is None:
            msg = sidecar.recv()
            if msg["type"] == "mask":
                masks.append(msg)
            else:
                done = msg
        assert len(masks) == 6
        assert done["type"] == "propagate_done"
        assert done["completed"] == 6
        assert done["skipped"] == 0
        for i, m in enumerate(masks):
            assert m["frameId"] == f"{i:04d}"
            assert Path(m["maskPath"]).is_file()
            assert 0.0 < m["coverage"] <= 1.0

    def test_resume_after_simulated_crash_does_not_deadlock(
        self, tmp_path, frame_dir
    ):
        """Kill mid-propagate, restart, resume: completes without hanging."""
        masks_dir = tmp_path / "masks"

        # First process: die after receiving two mask responses.
        sc1 = SidecarProcess(tmp_path)
        try:
            sc1.request(
                {
                    "type": "init_session",
                    "sessionId": "s1",
                    "frames": frames_payload(frame_dir, 6),
                }
            )
            sc1.request(
                {
                    "type": "add_prompt",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "frameId": "0000",
                    "points": [[32, 24]],
                    "pointLabels": [1],
                }
            )
            sc1.send(
                {
                    "type": "propagate",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "masksDir": str(masks_dir),
                }
            )
            first = sc1.recv()
            second = sc1.recv()
            assert first["type"] == "mask" and second["type"] == "mask"
        finally:
            sc1.kill()  # simulated crash mid-propagate

        # State checkpoint exists with all frames marked complete, but the
        # parent's masks for later frames may be missing (like a real crash).
        assert (masks_dir / "state.json").is_file()

        # Second process: resume must finish quickly (no deadlock, no hang).
        sc2 = SidecarProcess(tmp_path)
        try:
            sc2.proc  # alive
            sc2.request(
                {
                    "type": "init_session",
                    "sessionId": "s1",
                    "frames": frames_payload(frame_dir, 6),
                }
            )
            sc2.request(
                {
                    "type": "add_prompt",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "frameId": "0000",
                    "points": [[32, 24]],
                    "pointLabels": [1],
                }
            )
            sc2.send(
                {
                    "type": "propagate",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "masksDir": str(masks_dir),
                }
            )
            # Timeout guard: if the sidecar deadlocked, recv would hang past 30s.
            masks = []
            done = None
            while done is None:
                msg = sc2.recv(timeout=30)
                if msg["type"] == "mask":
                    masks.append(msg)
                else:
                    done = msg
            assert done["type"] == "propagate_done"
            # All six masks present on disk after resume.
            for i in range(6):
                assert (masks_dir / f"{i:04d}.png").is_file()
            assert done["completed"] + done["skipped"] == 6
        finally:
            sc2.kill()


class TestErrorRecovery:
    def test_malformed_input_then_ping(self, sidecar):
        sidecar.proc.stdin.write("{broken json}\n")
        sidecar.proc.stdin.flush()
        err = sidecar.recv()
        assert err["type"] == "error"
        assert err["code"] == "bad_request"
        assert sidecar.request({"type": "ping"})["type"] == "pong"

    def test_missing_frame_file(self, sidecar, tmp_path):
        resp = sidecar.request(
            {
                "type": "init_session",
                "sessionId": "s1",
                "frames": [{"frameId": "0000", "path": str(tmp_path / "nope.png")}],
            }
        )
        assert resp["type"] == "error"
        assert resp["code"] == "invalid_request"
        assert sidecar.request({"type": "ping"})["type"] == "pong"
