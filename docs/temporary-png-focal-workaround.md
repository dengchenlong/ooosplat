# Temporary PNG/XMP focal-length workaround (#73)

Implemented on `main`, 2026-10-19. This is an application-side compatibility workaround, not a new focal-length policy. No image rewriting, engine rebuild, setting, dialog or frontend state is added. Issue: https://github.com/ooolabdev/ooosplat/issues/73.

## Changes and applicability

- `src-tauri/src/video/temporary_png_focal_workaround.rs`: the internal switch, frozen affected runtime identity, first-work-image selection, bounded PNG/XMP reader, rational arithmetic, input checkpoint and regression tests.
- `src-tauri/src/video/mod.rs`: one module declaration.
- `src-tauri/src/engines/colmap.rs`: optional camera parameters in the ordinary extraction entrypoints and argument builder. Existing explicit values are forwarded unchanged; incremental/existing-camera and panorama paths do not request this workaround.
- `src-tauri/src/pipeline/runner.rs`: detection before checkpoint reuse, parameter forwarding and narrowly scoped cache archiving, with interruption/failure tests.
- `src-tauri/src/project/metadata.rs`: optional `temporaryPngFocal` checkpoint. Old state files remain readable; the project schema/version is unchanged.
- `src-tauri/Cargo.toml` and `Cargo.lock`: directly reference the already-locked `crc32fast` dependency to check relevant PNG chunks. No new model/runtime or frontend dependency.

Only ordinary image-sequence initialization with `SIMPLE_RADIAL` and `single_camera=1`, without explicit parameters, calibration or an existing camera, is eligible. The actual executable's help must identify COLMAP 4.2.1 / bd1fcf654d2dd8fefa1466999c190a246f83f4b9, and its installed BUILD-INFO / component records must identify runtime revision 1 and OpenImageIO 3.1.14.0. Missing, different or unverifiable identities disable the workaround.

The first actual work image follows COLMAP's lexical sorting, including sorting an explicit image list. A JPG first image is not replaced by a later PNG. Only this image's metadata is read; no batch metadata scan is added.

Supported PNGs contain a single unambiguous pre-IDAT `XML:com.adobe.xmp` iTXt packet (uncompressed or zlib), with no eXIf or alternative textual EXIF/focal source. XMP must provide valid focal length, a focal-plane X-resolution fraction with denominator other than one, unit 2/3/4/5 and dimensions consistent with IHDR. At least one complete TIFF or EXIF dimension pair must match. Orientation is absent or 1. Namespace prefixes may vary; attributes and simple element values are supported. A 35mm-equivalent field, even zero, conservatively disables this narrow workaround.

The reader seeks past pixel/ICC data without decoding or changing it. Metadata and decompressed output share a 64 KiB budget. It rejects truncation, relevant CRC failures, duplicate/ambiguous packets or fields, DTD/entities, malformed or overflowing numbers, zero denominators, non-positive or non-finite results. It does not use DPI or create a fallback focal estimate. The output is `[f, actual_width/2, actual_height/2, 0]`; feature extraction's smaller SIFT image size does not scale these coordinates. BA optimization settings remain unchanged.

## Cache behavior

The optional record binds the temporary policy version, first work image name, SHA-256, actual dimensions and camera parameters. The image hash is computed only after runtime and metadata eligibility checks.

For affected inputs only, an absent/different record forces old feature, match, reconstruction and Brush checkpoints to become non-reusable. That barrier is atomically persisted before moving any caches. Nonempty `work/colmap` and `work/brush` directories are renamed into `work/cache-archive/temporary-png-focal-v1-<uuid>/`; the new record is saved afterward. Failure or interruption does not certify the old database. Identical records allow normal reuse; non-applicable detection does not invalidate caches.

Pictures, hard links, masks, published `final.ply`, editing data and export files are not archived or rewritten by this hook. Completed projects are not automatically reopened/reconstructed. Existing user projects are not scanned or mutated at application startup. No database camera values are patched in place.

## Verification

Windows, bundled runtime, real `003A9246.JPG` and `003A9246.png`: fresh isolated directories and databases, CPU feature extraction only, SIFT maximum size 512 / maximum features 512. No matching, mapping or Brush training. The original 8192×5464 camera coordinates remain intact.

| Input | Workaround | Database camera parameters |
| --- | --- | --- |
| Original JPG | Not applied | `22728.65980253445,4096,2732,0` |
| Original PNG | Automatically applied | `22728.65997458563,4096,2732,0` |

Both cameras have model ID 2 and `prior_focal_length=1`. The PNG result is the f64 rational calculation; the small difference from the JPG's OIIO float32 path is expected.

Original and work-copy SHA-256 before/after match:

- JPG: `99218f6d9c45f3627c4d231e3b94a7bd392ff127a2af63fc17609b93f406f6c4`
- PNG: `00a245b06afea62fcd94d5186c9211dc53eb254ad0fdb5af433ed519d104f0e8`

Evidence: `.tmp/png-focal-workaround-H5xqbK/result.json`, with separate JPG/PNG databases and logs. The existing `.tmp/focal-diagnosis-20261019/` databases were not reused or modified. The native test is opt-in (`bundled_png_focal_regression`) and was explicitly executed successfully, not merely counted as ignored.

Automated coverage includes rational arithmetic, units, namespaces, malformed/uncertain metadata, budgets, lexical/image-list selection, explicit calibration guards, hard-link read-only behavior, old/new state round trips, cache reuse/invalidation and a durable barrier after archive failure. Frontend 453 tests plus 6 diagnostics, Rust 407 tests, frontend build, formatting and Clippy passed. Two existing Windows symlink-permission tests were explicitly excluded; the default Rust run ignores 17 opt-in tests. No full-batch reconstruction or macOS/Linux native regression was run.

## Removal

After upgrading the runtime and passing the **original unchanged PNG** regression without a camera override:

1. Set the module's single `ENABLED` constant to `false` to disable detection immediately.
2. For complete removal, delete the module and its declaration, Runner preflight/parameter selection and cache hook, optional checkpoint field/initializer, and patch-specific tests. Existing state files tolerate removal of this optional field.
3. Keep the generic optional COLMAP camera-parameter argument if useful, passing `None` where the workaround used to supply it. Remove the direct `crc32fast` dependency if unused elsewhere.

Do not broaden the frozen version condition or remove the compatibility patch solely because a version number changed; validate the original file first.
