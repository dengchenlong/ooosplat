# SAM2 sidecar (Windows)

SAM 2.1 runs as a local Python sidecar process, driven over stdin/stdout with
a JSON-lines protocol (see `sidecar/`). All inference is local: model weights
are distributed as files, and no cloud inference API is ever contacted.

## Layout

- `sidecar/` — the Python package (`sam2_sidecar`) with the JSON-lines server,
  protocol codec, mask writer, synthetic test backend and the real SAM2
  backend glue (`sam2_video.py`, lazily imported).
- Model weights are **not** stored in this repository. They are downloaded
  from the official Meta distribution and installed under
  `D:\ooosplat-engines\sam2\` (configurable), outside both the repo and the
  system drive.

## Weight download (official source, local distribution)

```powershell
$dest = "D:\ooosplat-engines\sam2\checkpoints"
New-Item -ItemType Directory -Force $dest | Out-Null
Invoke-WebRequest `
  -Uri "https://dl.fbaipublicfiles.com/segment_anything_2/092824/sam2.1_hiera_tiny.pt" `
  -OutFile (Join-Path $dest "sam2.1_hiera_tiny.pt")
```

`engines/sam2/manifest.json` pins the expected SHA-256 and size; verify with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify-sam2.ps1
```

The pinned file is the official `sam2.1_hiera_tiny.pt` (156,008,466 bytes,
SHA-256 recorded in `engines/sam2/manifest.json`). Only tiny/small/base-plus
builds from `dl.fbaipublicfiles.com` are accepted; a hash mismatch blocks
startup rather than being ignored.

## Sidecar runtime

The sidecar needs `torch` (CUDA build for GPU machines) and the `sam2`
package. For development, from `engines/sam2/sidecar`:

```text
uv sync                       # dev/test environment (no torch)
uv run pytest                 # protocol + subprocess tests (synthetic backend)
uv run python -m sam2_sidecar --backend sam2 --checkpoint D:\ooosplat-engines\sam2\checkpoints\sam2.1_hiera_tiny.pt
```

The production runtime (embedded Python + torch + sam2, NVIDIA-only) is a
later distribution task; `engines/sam2/manifest.json` already records the
required components and the weight pins so `verify-sam2.ps1` can validate an
install. Protocol tests run everywhere using `--backend synthetic`, which
needs no GPU and no weights.

## Protocol

One UTF-8 JSON object per line. Requests: `hello`, `ping`, `init_session`
(frame list with ids + paths), `add_prompt` (point and/or box on an anchor
frame, per object), `propagate` (streams one `mask` response per frame, then
`propagate_done`), `shutdown`. Errors are `error` responses with a stable
`code`; the server never dies on malformed input. Propagation checkpoints
`state.json` atomically after every frame; a restarted sidecar resumes by
skipping masks that already exist and restarts cleanly when the frame set or
prompt changes (`framesHash` mismatch).

## License

SAM 2.1 code and weights are Apache-2.0 (`licenses/SAM2-Apache-2.0.txt`).
