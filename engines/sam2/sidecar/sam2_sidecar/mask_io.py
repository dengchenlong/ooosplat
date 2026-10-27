"""Pure-stdlib PNG I/O for binary masks (8-bit grayscale, 0/255).

Keeps the synthetic backend dependency-free; the real SAM2 backend converts
its boolean arrays to the same row format.
"""

import struct
import zlib
from pathlib import Path

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


class NotAPngError(ValueError):
    pass


def png_size(path) -> tuple[int, int]:
    """Return (width, height) parsed from a PNG IHDR header."""
    data = Path(path).read_bytes()[:33]
    if len(data) < 33 or data[:8] != PNG_SIGNATURE:
        raise NotAPngError(f"not a PNG file: {path}")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def read_mask_png(path) -> tuple[int, int, list[bytes]]:
    """Read a grayscale PNG mask into ``(width, height, rows)``.

    Accepts what :func:`write_mask_png` emits (8-bit grayscale, filter type 0)
    plus 1-bit and 16-bit grayscale, so historical mask directories produced
    by other tools still load. Colour images are rejected rather than
    silently reinterpreted: an RGB screenshot is not a mask.

    Row values are returned as stored (masks are 0/255; anything above 127 is
    treated as foreground by the post-processor).
    """
    data = Path(path).read_bytes()
    if data[:8] != PNG_SIGNATURE:
        raise NotAPngError(f"not a PNG file: {path}")
    if len(data) < 33:
        raise NotAPngError(f"truncated PNG file: {path}")

    header = None
    idat: list[bytes] = []
    pos = 8
    while pos + 8 <= len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        tag = data[pos + 4 : pos + 8]
        payload = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if tag == b"IHDR":
            header = struct.unpack(">IIBBBBB", payload)
        elif tag == b"IDAT":
            idat.append(payload)
        elif tag == b"IEND":
            break
    if header is None or not idat:
        raise NotAPngError(f"PNG has no header or image data: {path}")

    width, height, depth, color_type, _compression, _filter, interlace = header
    if color_type != 0:
        raise NotAPngError(
            f"only grayscale PNG masks are supported (color type {color_type}): {path}"
        )
    if interlace != 0:
        raise NotAPngError(f"interlaced PNG masks are not supported: {path}")
    if depth not in (1, 8, 16):
        raise NotAPngError(f"unsupported PNG bit depth {depth}: {path}")

    try:
        raw = zlib.decompress(b"".join(idat))
    except zlib.error as exc:  # pragma: no cover - corrupt files only
        raise NotAPngError(f"corrupt PNG image data: {path}") from exc

    stride = (width * depth + 7) // 8
    if len(raw) < height * (stride + 1):
        raise NotAPngError(f"truncated PNG image data: {path}")
    rows = _unfilter(raw, height, stride, bpp=1 if depth <= 8 else 2)

    if depth == 8:
        return width, height, rows
    if depth == 16:
        # 0/65535 masks: the high byte already carries the 0/255 value.
        return width, height, [bytes(row[i] for i in range(0, len(row), 2)) for row in rows]
    # 1-bit: one bit per pixel, MSB first, each row padded to a byte boundary.
    unpacked = []
    for row in rows:
        bits = bytearray(width)
        for x in range(width):
            bits[x] = 255 if (row[x >> 3] >> (7 - (x & 7))) & 1 else 0
        unpacked.append(bytes(bits))
    return width, height, unpacked


def _unfilter(raw: bytes, height: int, stride: int, bpp: int) -> list[bytes]:
    """Undo PNG scanline filtering (types 0-4)."""
    filters = raw[0 :: stride + 1]
    if not any(filters):
        # Fast path: every scanline uses filter type 0, which is what
        # write_mask_png emits, so rows are already contiguous.
        return [raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)] for y in range(height)]

    out: list[bytes] = []
    prev = bytearray(stride)
    for y in range(height):
        offset = y * (stride + 1)
        ftype = raw[offset]
        line = bytearray(raw[offset + 1 : offset + 1 + stride])
        if ftype == 0:
            pass
        elif ftype == 1:  # Sub
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:  # Average
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:  # Paeth
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                up = prev[i]
                up_left = prev[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + _paeth(left, up, up_left)) & 0xFF
        else:  # pragma: no cover - invalid PNG
            raise NotAPngError(f"unknown PNG filter type {ftype}")
        out.append(bytes(line))
        prev = line
    return out


def _paeth(left: int, up: int, up_left: int) -> int:
    estimate = left + up - up_left
    d_left = abs(estimate - left)
    d_up = abs(estimate - up)
    d_up_left = abs(estimate - up_left)
    if d_left <= d_up and d_left <= d_up_left:
        return left
    if d_up <= d_up_left:
        return up
    return up_left


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
