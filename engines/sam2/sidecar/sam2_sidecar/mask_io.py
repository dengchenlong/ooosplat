"""Pure-stdlib PNG I/O for binary masks (8-bit grayscale, 0/255).

Keeps the synthetic backend dependency-free; the real SAM2 backend converts
its boolean arrays to the same row format.
"""

import struct
import zlib
from pathlib import Path


class NotAPngError(ValueError):
    pass


def png_size(path) -> tuple[int, int]:
    """Return (width, height) parsed from a PNG IHDR header."""
    data = Path(path).read_bytes()[:33]
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise NotAPngError(f"not a PNG file: {path}")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def rows_from_rects(rects, width: int, height: int) -> list[bytes]:
    """Build 0/255 row bytes from (x0, y0, x1, y1) inclusive rects."""
    rows = []
    for y in range(height):
        row = bytearray(width)
        for x0, y0, x1, y1 in rects:
            if y0 <= y <= y1:
                start = max(0, min(x0, width - 1))
                stop = max(0, min(x1, width - 1))
                for x in range(start, stop + 1):
                    row[x] = 255
        rows.append(bytes(row))
    return rows


def coverage_of_rows(rows: list[bytes]) -> float:
    if not rows:
        return 0.0
    covered = sum(row.count(255) for row in rows)
    return covered / (len(rows) * len(rows[0]))


def write_mask_png(path, width: int, height: int, rows: list[bytes]) -> None:
    """Write an 8-bit grayscale PNG. Atomic: temp file + rename."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if len(rows) != height or any(len(r) != width for r in rows):
        raise ValueError("mask rows do not match width/height")

    def chunk(tag: bytes, data: bytes) -> bytes:
        payload = tag + data
        return (
            struct.pack(">I", len(data))
            + payload
            + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    raw = b"".join(b"\x00" + row for row in rows)
    body = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(body)
    tmp.replace(path)
