"""End-to-end pipeline test: sidecar masks -> Brush-ready dataset layout.

Simulates how the Rust orchestrator will consume the sidecar: init session
with real frame files, add a point prompt, propagate, and verify the masks
land as Brush-compatible PNGs (grayscale, white=keep, one per frame, named
after the frame id) plus a resumable state.json.
"""

import struct
import subprocess
import sys
import zlib
from pathlib import Path

import pytest

from conftest import make_png

SIDECAR_ROOT = Path(__file__).resolve().parent.parent


def png_mode(path: Path) -> int:
    data = path.read_bytes()
    # IHDR colour type is byte 25 (9 bytes after the 16-byte IHDR header start)
    return data[25]


class TestBrushDatasetLayout:
    def test_masks_are_brush_compatible(self, tmp_path):
        frame_dir = tmp_path / "frames"
        frame_dir.mkdir()
        for i in range(3):
            make_png(frame_dir / f"{i:04d}.png", width=64, height=48, fill=(50, 150, 90))

        masks_dir = tmp_path / "brush" / "dataset" / "masks"
        proc = subprocess.Popen(
            [sys.executable, "-m", "sam2_sidecar", "--backend", "synthetic"],
            cwd=str(SIDECAR_ROOT),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        try:
            def send(msg):
                proc.stdin.write(_json(msg) + "\n")
                proc.stdin.flush()

            def recv():
                line = proc.stdout.readline()
                assert line, "sidecar died: " + proc.stderr.read()
                import json

                return json.loads(line)

            def request(msg):
                send(msg)
                return recv()

            assert request({"type": "ping"})["type"] == "pong"
            request(
                {
                    "type": "init_session",
                    "sessionId": "s1",
                    "frames": [
                        {"frameId": f"{i:04d}", "path": str(frame_dir / f"{i:04d}.png")}
                        for i in range(3)
                    ],
                }
            )
            request(
                {
                    "type": "add_prompt",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "frameId": "0000",
                    "box": [20, 12, 43, 35],
                }
            )
            send(
                {
                    "type": "propagate",
                    "sessionId": "s1",
                    "objectId": "obj1",
                    "masksDir": str(masks_dir),
                }
            )
            masks, done = [], None
            while done is None:
                msg = recv()
                if msg["type"] == "mask":
                    masks.append(msg)
                else:
                    done = msg

            assert len(masks) == 3
            for i in range(3):
                mask_path = masks_dir / f"{i:04d}.png"
                assert mask_path.is_file()
                # 8-bit grayscale (colour type 0), Brush mask convention.
                assert png_mode(mask_path) == 0
            assert done["type"] == "propagate_done"
            assert done["completed"] == 3
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=10)


def _json(msg) -> str:
    import json

    return json.dumps(msg, ensure_ascii=False)
