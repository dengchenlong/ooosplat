"""Acceptance check for the issue #11 `assets/` publication layout.

Validates a project's published artefacts without any of the Rust code that
wrote them, so it can catch a manifest that disagrees with the files on disk:

    python scripts/verify-assets.py <project_root>

Checks
  1. assets/manifest.json parses and declares version 1 with a `source` block
     naming a final.ply that exists at the project root.
  2. Every `assets[]` entry names a file inside assets/, the file exists, and
     its size and SHA-256 match the manifest.
  3. Each entry matches its kind: a ply entry carries splatCount, a glb entry
     carries vertexCount + triangleCount (+ units / scaleFactor) and no
     splatCount.
  4. The entries are strictly sorted by (kind, index, format) and unique on the
     same triple, so two runs over the same inputs produce identical bytes.
  5. source.finalPlySha256 and source.splatCount equal a fresh read of final.ply,
     and the split is exhaustive: every splat cloud's count plus the others
     equals the source count.
  6. `background.ply` quality: every row is a byte-identical final.ply row, in
     final.ply order (the split subset never rewrites or reorders a splat).

Exit code 0 means every check passed; 1 means a check failed (reason printed).
"""

import hashlib
import json
import os
import struct
import sys
from typing import NoReturn

PLY_MAGIC = b"ply\n"
REQUIRED_PROPERTIES = ("x", "y", "z", "f_dc_0", "opacity", "scale_0", "rot_0")


def fail(message: str) -> NoReturn:
    print(f"FAIL: {message}")
    sys.exit(1)


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_ply(path):
    """(splat_count, stride, rows) using only the header's element/property lines."""
    with open(path, "rb") as handle:
        blob = handle.read()
    if not blob.startswith(PLY_MAGIC):
        fail(f"{path} is not a PLY file")
    marker = blob.find(b"end_header\n")
    if marker < 0:
        fail(f"{path} has no end_header")
    end = marker + len(b"end_header\n")
    lines = [line.strip() for line in blob[:end].decode("ascii").splitlines()]
    if "format binary_little_endian 1.0" not in lines:
        fail(f"{path} is not binary_little_endian")
    count, stride, names = None, 0, []
    for line in lines:
        fields = line.split()
        if fields[:2] == ["element", "vertex"]:
            count = int(fields[2])
        elif fields[:1] == ["property"]:
            if fields[1] != "float":
                fail(f"{path} property {fields[2]} is not float")
            names.append(fields[2])
            stride += 4
    if count is None or stride == 0:
        fail(f"{path} declares no vertex element")
    for name in REQUIRED_PROPERTIES:
        if name not in names:
            fail(f"{path} is missing the Gaussian property {name}")
    payload = blob[end:]
    if len(payload) != count * stride:
        fail(f"{path} payload is {len(payload)} bytes, header implies {count * stride}")
    rows = [payload[i * stride : (i + 1) * stride] for i in range(count)]
    return count, stride, rows


def read_source_count(path):
    count, _, _ = read_ply(path)
    return count


def main(root):
    assets = os.path.join(root, "assets")
    manifest_path = os.path.join(assets, "manifest.json")
    if not os.path.isfile(manifest_path):
        fail(f"{manifest_path} does not exist")
    with open(manifest_path, encoding="utf-8") as handle:
        manifest = json.load(handle)

    if manifest.get("version") != 1:
        fail(f"manifest version is {manifest.get('version')!r}, expected 1")
    if not isinstance(manifest.get("generatedAtMs"), int):
        fail("generatedAtMs is not an integer")

    source = manifest.get("source") or {}
    final_relative = source.get("finalPly")
    if not final_relative or os.path.isabs(final_relative):
        fail(f"source.finalPly must be a relative path, got {final_relative!r}")
    final_path = os.path.join(root, final_relative)
    if not os.path.isfile(final_path):
        fail(f"source.finalPly {final_relative} does not exist under {root}")
    if sha256_file(final_path) != source.get("finalPlySha256"):
        fail("source.finalPlySha256 does not match final.ply")
    final_count = read_source_count(final_path)
    if source.get("splatCount") != final_count:
        fail(
            f"source.splatCount is {source.get('splatCount')}, final.ply has {final_count}"
        )

    entries = manifest.get("assets")
    if not isinstance(entries, list) or not entries:
        fail("manifest has no assets[] entries")

    keys = []
    ply_splats = 0
    for entry in entries:
        name = entry.get("file")
        if not name or os.path.isabs(name) or os.sep in name or "/" in name:
            fail(f"entry file {name!r} must be a bare name inside assets/")
        path = os.path.join(assets, name)
        if not os.path.isfile(path):
            fail(f"entry {name} names a file that does not exist")
        if entry.get("fileSize") != os.path.getsize(path):
            fail(f"entry {name} fileSize disagrees with the file on disk")
        if entry.get("sha256") != sha256_file(path):
            fail(f"entry {name} sha256 disagrees with the file on disk")

        kind, index, fmt = entry.get("kind"), entry.get("index"), entry.get("format")
        if kind not in ("object", "background"):
            fail(f"entry {name} has unknown kind {kind!r}")
        if fmt not in ("ply", "glb"):
            fail(f"entry {name} has unknown format {fmt!r}")
        if fmt == "ply":
            if "splatCount" not in entry:
                fail(f"ply entry {name} has no splatCount")
            if "vertexCount" in entry or "triangleCount" in entry:
                fail(f"ply entry {name} carries mesh counts")
            expected = read_source_count(path)
            if entry["splatCount"] != expected:
                fail(f"entry {name} splatCount is {entry['splatCount']}, file has {expected}")
            ply_splats += entry["splatCount"]
        else:
            for field in ("vertexCount", "triangleCount", "units", "scaleFactor"):
                if field not in entry:
                    fail(f"glb entry {name} has no {field}")
            if "splatCount" in entry:
                fail(f"glb entry {name} carries a splatCount")
        keys.append((kind, index, fmt))

    if keys != sorted(keys):
        fail(f"entries are not sorted by (kind, index, format): {keys}")
    if len(set(keys)) != len(keys):
        fail(f"entries are not unique on (kind, index, format): {keys}")

    if ply_splats != final_count:
        fail(f"ply entries sum to {ply_splats} splats, final.ply has {final_count}")

    # background.ply must be a byte-exact, order-preserving subset of final.ply.
    _, final_stride, final_rows = read_ply(final_path)
    background_path = os.path.join(assets, "background.ply")
    if os.path.isfile(background_path):
        _, background_stride, background_rows = read_ply(background_path)
        if background_stride != final_stride:
            fail("background.ply stride differs from final.ply")
        final_set = set(final_rows)
        missing = [row for row in background_rows if row not in final_set]
        if missing:
            fail(
                f"{len(missing)} background.ply rows are not byte-identical final.ply rows"
            )
        # Order: walking final.ply once must yield background_rows in sequence.
        iterator = iter(background_rows)
        wanted = next(iterator, None)
        for row in final_rows:
            if wanted is not None and row == wanted:
                wanted = next(iterator, None)
        if wanted is not None:
            fail("background.ply rows are not in final.ply order")
        print(
            f"background.ply  {len(background_rows)} rows, all byte-identical "
            "final.ply rows in order (quality not below final.ply)"
        )

    print(f"manifest        {manifest_path} (version {manifest['version']})")
    print(f"source          {final_relative} sha256 OK, {final_count} splats")
    for entry in entries:
        print(
            "entry           %-10s idx %d %-3s %-16s %8d bytes  %s"
            % (
                entry["kind"],
                entry["index"],
                entry["format"],
                entry["file"],
                entry["fileSize"],
                entry["sha256"][:16],
            )
        )
    print(f"ply splats      {ply_splats} == final {final_count}")
    print("OK: manifest, hashes, sizes, ordering, coverage and background quality all valid")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1])
