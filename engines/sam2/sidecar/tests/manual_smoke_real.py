"""One-off CPU smoke of the real SAM2 backend (frame-by-frame video predictor)."""

import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

SIDECAR = Path(__file__).resolve().parents[1]
CKPT = r"D:\ooosplat-engines\sam2\checkpoints\sam2.1_hiera_tiny.pt"

tmp = Path(tempfile.mkdtemp(prefix="sam2-real-"))
frames = tmp / "frames"
# 4 frames, 96x72: colored background + a red square that moves slightly
for i in range(4):
    import struct
    import zlib

    W, H = 96, 72
    off = i * 3  # square drifts right 3px/frame
    rows = b""
    for y in range(H):
        row = bytearray()
        for x in range(W):
            if 20 + off <= x < 45 + off and 15 <= y < 45:
                row += bytes((220, 30, 30))
            else:
                row += bytes((40, 90, 160))
        rows += b"\x00" + bytes(row)

    def chunk(tag, data):
        payload = tag + data
        return (
            struct.pack(">I", len(data))
            + payload
            + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0)
    (frames).mkdir(parents=True, exist_ok=True)
    (frames / f"{i:04d}.png").write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )

proc = subprocess.Popen(
    [sys.executable, "-m", "sam2_sidecar", "--backend", "sam2", "--checkpoint", CKPT],
    cwd=str(SIDECAR),
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
    encoding="utf-8",
)


def send(m):
    proc.stdin.write(json.dumps(m) + "\n")
    proc.stdin.flush()


def recv():
    line = proc.stdout.readline()
    assert line, (
        "sidecar exited: " + proc.stderr.read()
        if proc.poll() is not None
        else "sidecar exited early"
    )
    return json.loads(line)


t0 = time.monotonic()
send({"type": "hello", "protocolVersion": 1})
h = recv()
print("hello:", h)
assert h["backend"] == "sam2"
send(
    {
        "type": "init_session",
        "sessionId": "real",
        "frames": [{"frameId": f"{i:04d}", "path": str(frames / f"{i:04d}.png")} for i in range(4)],
    }
)
r = recv()
print("session:", r)
send(
    {
        "type": "add_prompt",
        "sessionId": "real",
        "objectId": "square",
        "frameId": "0000",
        "points": [[32, 30]],
        "pointLabels": [1],
    }
)
print("prompt:", recv())
send(
    {"type": "propagate", "sessionId": "real", "objectId": "square", "masksDir": str(tmp / "masks")}
)
n = 0
while True:
    m = recv()
    if m["type"] == "mask":
        n += 1
        print(f"mask {m['frameId']} coverage={m['coverage']:.3f}")
    elif m["type"] == "propagate_done":
        print("done:", m)
        break
    else:
        raise AssertionError(m)
send({"type": "shutdown"})
assert recv()["type"] == "bye"
assert proc.wait(timeout=20) == 0
# sanity: masks should track the drifting square (coverage ~ 25*30/6912 ≈ 0.108)
print(f"SAM2 CPU SMOKE OK: {n} masks in {time.monotonic() - t0:.1f}s -> {tmp / 'masks'}")
