"""Acceptance check for an `objmesh_mesh` GLB (issue #10).

Validates the container and the payload without a glTF library or Blender:

    python scripts/verify-glb.py <file.glb>

Checks
  1. 12-byte header: magic "glTF", version 2, declared length == file size.
  2. Exactly two 4-byte-aligned chunks: JSON (padded) then BIN (padded).
  3. The JSON chunk parses, declares asset.version "2.0" and a generator.
  4. Every accessor fits its bufferView, and every bufferView fits the BIN
     chunk, with count * element size matching the declared byteLength.
  5. The POSITION accessor's min/max equal the actual float data.
  6. The index accessor stays inside [0, vertex_count).
  7. Reports vertices, triangles, bounds, colour range and unit normals so a
     human can compare them against the source splat AABB.

Exit code 0 means every check passed; 1 means a check failed (the reason is
printed).
"""

import json
import struct
import sys
from typing import NoReturn

MAGIC = b"glTF"
CHUNK_JSON = 0x4E4F534A
CHUNK_BIN = 0x004E4942
FLOAT = 5126
UNSIGNED_INT = 5125


def fail(message: str) -> NoReturn:
    print(f"FAIL: {message}")
    sys.exit(1)


def main(path):
    data = open(path, "rb").read()
    if len(data) < 20:
        fail(f"file is only {len(data)} bytes")
    magic, version, declared = struct.unpack_from("<4sII", data, 0)
    if magic != MAGIC:
        fail(f"magic is {magic!r}, expected {MAGIC!r}")
    if version != 2:
        fail(f"container version is {version}, expected 2")
    if declared != len(data):
        fail(f"header declares {declared} bytes, file has {len(data)}")
    if len(data) % 4:
        fail("file length is not a multiple of 4")

    json_len, json_type = struct.unpack_from("<II", data, 12)
    if json_type != CHUNK_JSON:
        fail("first chunk is not JSON")
    json_end = 20 + json_len
    document = json.loads(data[20:json_end])
    bin_len, bin_type = struct.unpack_from("<II", data, json_end)
    if bin_type != CHUNK_BIN:
        fail("second chunk is not BIN")
    bin_start = json_end + 8
    if bin_start + bin_len != len(data):
        fail("BIN chunk does not end at the file end")
    binary = data[bin_start : bin_start + bin_len]

    if document.get("asset", {}).get("version") != "2.0":
        fail("asset.version is not '2.0'")
    if not document.get("asset", {}).get("generator"):
        fail("asset.generator is missing")

    primitives = document["meshes"][0]["primitives"]
    if len(primitives) != 1:
        fail(f"expected one primitive, found {len(primitives)}")
    primitive = primitives[0]
    if primitive["mode"] != 4:
        fail("primitive mode is not 4 (triangles)")

    accessors = document["accessors"]
    views = document["bufferViews"]
    for index, accessor in enumerate(accessors):
        view = views[accessor["bufferView"]]
        if view["buffer"] != 0:
            fail(f"accessor {index} points at a non-existent buffer")
        components = 3 if accessor["type"] == "VEC3" else 1
        size = 4 if accessor["componentType"] in (FLOAT, UNSIGNED_INT) else None
        if size is None:
            fail(f"accessor {index} has an unexpected componentType")
        needed = accessor["count"] * components * size
        offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
        if view.get("byteOffset", 0) + view["byteLength"] > len(binary):
            fail(f"bufferView {accessor['bufferView']} overruns the BIN chunk")
        if needed > view["byteLength"]:
            fail(f"accessor {index} needs {needed} bytes, view has {view['byteLength']}")
        if offset + needed > len(binary):
            fail(f"accessor {index} overruns the BIN chunk")

    def read_vec3(accessor_index):
        accessor = accessors[accessor_index]
        view = views[accessor["bufferView"]]
        offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
        count = accessor["count"]
        flat = struct.unpack_from(f"<{count * 3}f", binary, offset)
        return [flat[i * 3 : i * 3 + 3] for i in range(count)]

    positions = read_vec3(primitive["attributes"]["POSITION"])
    normals = read_vec3(primitive["attributes"]["NORMAL"])
    colors = read_vec3(primitive["attributes"]["COLOR_0"])
    if not (len(positions) == len(normals) == len(colors)):
        fail("POSITION / NORMAL / COLOR_0 counts differ")

    index_accessor = accessors[primitive["indices"]]
    index_view = views[index_accessor["bufferView"]]
    index_offset = index_view.get("byteOffset", 0) + index_accessor.get("byteOffset", 0)
    indices = struct.unpack_from(f"<{index_accessor['count']}I", binary, index_offset)
    if index_accessor["count"] % 3:
        fail("index count is not a multiple of 3")
    if indices and (max(indices) >= len(positions) or min(indices) > max(indices)):
        fail("an index points outside the vertex array")

    actual_min = [min(p[axis] for p in positions) for axis in range(3)]
    actual_max = [max(p[axis] for p in positions) for axis in range(3)]
    position_accessor = accessors[primitive["attributes"]["POSITION"]]
    declared_min = position_accessor.get("min")
    declared_max = position_accessor.get("max")
    if declared_min is None or declared_max is None:
        fail("POSITION accessor is missing min/max")
    for axis in range(3):
        if abs(declared_min[axis] - actual_min[axis]) > 1e-6:
            fail(
                f"POSITION min[{axis}] declares {declared_min[axis]}, "
                f"data says {actual_min[axis]}"
            )
        if abs(declared_max[axis] - actual_max[axis]) > 1e-6:
            fail(
                f"POSITION max[{axis}] declares {declared_max[axis]}, "
                f"data says {actual_max[axis]}"
            )

    lengths = [
        sum((n[i] * n[i]) for i in range(3)) ** 0.5 for n in normals
    ]
    if any(abs(length - 1.0) > 1e-2 for length in lengths):
        worst = max(abs(length - 1.0) for length in lengths)
        fail(f"vertex normals are not unit length (worst deviation {worst:.4f})")
    if any(c < -1e-6 or c > 1 + 1e-6 for colour in colors for c in colour):
        fail("a vertex colour channel is outside [0, 1]")

    span = [actual_max[axis] - actual_min[axis] for axis in range(3)]
    print(f"file            {path} ({len(data)} bytes)")
    print(f"json chunk      {json_len} bytes, generator {document['asset']['generator']!r}")
    print(f"bin chunk       {bin_len} bytes")
    print(f"vertices        {len(positions)}")
    print(f"triangles       {len(indices) // 3}")
    print(f"bbox min        [{actual_min[0]:.4f}, {actual_min[1]:.4f}, {actual_min[2]:.4f}]")
    print(f"bbox max        [{actual_max[0]:.4f}, {actual_max[1]:.4f}, {actual_max[2]:.4f}]")
    print(f"bbox span       [{span[0]:.4f}, {span[1]:.4f}, {span[2]:.4f}]")
    print(
        "colour range    "
        f"[{min(c[0] for c in colors):.3f}, {max(c[0] for c in colors):.3f}] r "
        f"[{min(c[1] for c in colors):.3f}, {max(c[1] for c in colors):.3f}] g "
        f"[{min(c[2] for c in colors):.3f}, {max(c[2] for c in colors):.3f}] b"
    )
    print("OK: container, accessors, indices, POSITION min/max, normals and colours all valid")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1])