"""CLI smoke test: drive the packaged entrypoint exactly as the Rust host will.

Run standalone (not part of pytest default collection) to validate the
console-script path:
    uv run python tests/smoke_cli.py
"""

import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SIDECAR_ROOT = Path(__file__).resolve().parent.parent


def make_png(path: Path, width=32, height=24):
    import struct
    import zlib

    raw = b"".join(b"\x00" + bytes((10, 200, 30)) * width for _ in range(height))

    def chunk(tag, data):
        payload = tag + data
        return (
            struct.pack(">I", len(data))
            + payload
            + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def main() -> int:
    tmp = Path(tempfile.mkdtemp(prefix="sam2-smoke-"))
    frames = tmp / "frames"
    for i in range(3):
        make_png(frames / f"{i:04d}.png")
    masks = tmp / "masks"

    proc = subprocess.Popen(
        [sys.executable, "-m", "sam2_sidecar", "--backend", "synthetic"],
        cwd=str(SIDECAR_ROOT),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )

    def send(msg):
        proc.stdin.write(json.dumps(msg) + "\n")
        proc.stdin.flush()

    def recv():
        line = proc.stdout.readline()
        assert line, "sidecar exited early"
        return json.loads(line)

    t0 = time.monotonic()
    send({"type": "hello", "protocolVersion": 1})
    hello = recv()
    assert hello["type"] == "hello" and hello["localOnly"] is True

    send(
        {
            "type": "init_session",
            "sessionId": "smoke",
            "frames": [
                {"frameId": f"{i:04d}", "path": str(frames / f"{i:04d}.png")} for i in range(3)
            ],
        }
    )
    ready = recv()
    assert ready["type"] == "session_ready" and ready["frameCount"] == 3

    send(
        {
            "type": "add_prompt",
            "sessionId": "smoke",
            "objectId": "bracelet",
            "frameId": "0000",
            "points": [[16, 12]],
            "pointLabels": [1],
        }
    )
    assert recv()["type"] == "prompt_accepted"

    send(
        {
            "type": "propagate",
            "sessionId": "smoke",
            "objectId": "bracelet",
            "masksDir": str(masks),
        }
    )
    count = 0
    while True:
        msg = recv()
        if msg["type"] == "mask":
            count += 1
            assert Path(msg["maskPath"]).is_file()
        elif msg["type"] == "propagate_done":
            assert msg["completed"] == 3
            break
        else:
            raise AssertionError(f"unexpected: {msg}")

    send({"type": "shutdown"})
    assert recv()["type"] == "bye"
    assert proc.wait(timeout=10) == 0

    print(f"SMOKE OK: {count} masks in {time.monotonic() - t0:.1f}s -> {masks}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
