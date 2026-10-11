"""Shared test fixtures: tiny PNG generation without heavy dependencies."""

import struct
import zlib
from pathlib import Path


def make_png(path, width=8, height=8, fill=(255, 0, 0)):
    """Write a minimal 8-bit RGB PNG at `path` filled with `fill`."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    raw = b"".join(b"\x00" + bytes(fill) * width for _ in range(height))

    def chunk(tag, data):
        payload = tag + data
        return (
            struct.pack(">I", len(data))
            + payload
            + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    body = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(body)
    return path


def png_size(path):
    """Read width/height from a PNG IHDR header."""
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    width, height = struct.unpack(">II", data[16:24])
    return width, height
