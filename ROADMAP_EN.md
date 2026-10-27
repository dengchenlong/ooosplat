# OOOSplat Roadmap

[中文](ROADMAP.md) | [English](ROADMAP_EN.md)

This roadmap describes OOOSplat's product direction and implementation priorities. P0–P3 indicate relative priority; they are not release numbers and do not guarantee delivery dates.

Current `main` version identifier: **0.7.0**. This page tracks source implementation; items marked “main” are not necessarily included in installers carrying the same version. Consult the corresponding release notes for shipped features.

## Product Principles

- **One-click workflow**: Continue reducing engine configuration and manual steps so users can turn source media directly into usable Gaussian Splatting results.
- **Local first**: Reconstruction, training, preview, and export should primarily use the user's own hardware without depending on cloud processing services.
- **Safe and non-destructive**: Keep source media and project data local by default, preserve original results during editing and export, and make file locations and processing state easy to trace.

## Priority Levels

- **P0 · Near-term focus**: Stability, speed, generation quality, and error handling.
- **P1 · High priority**: Evaluation infrastructure, data insights, capture guidance, and product experience.
- **P2 · Capability expansion**: New output formats, project import, and AI integrations.
- **P3 · Longer-term exploration**: Adoption and additional capture or usage scenarios.

## Planned Work

| Priority | Item | Goal | GitHub Issue |
| --- | --- | --- | --- |
| P0 | Very large PLY preview stability and performance | Eliminate memory peaks and WebView2 OOM crashes when loading PLY files larger than 1 GB, and improve large-scale Gaussian loading, editing, and cleanup. | [#21](https://github.com/ooolabdev/ooosplat/issues/21) |
| P0 | Performance optimization | Reduce time spent on media preparation, feature extraction, matching, reconstruction, and training while preserving result compatibility. | To be created |
| P0 | Generation quality optimization | Improve camera registration, geometric completeness, visual detail, and edge quality with measurable optimization strategies. | To be created |
| P0 | Better error guidance | Turn engine, media, disk, GPU, and reconstruction failures into clear actions and recovery guidance. | To be created |
| P0 | UI simplification | Reduce unnecessary information and interaction layers while unifying generation, history, preview, and editing workflows. | To be created |
| P1 | Gaussian generation benchmark | Establish reproducible datasets, hardware profiles, quality metrics, and timing metrics to compare speed, resource use, and output quality across releases. | To be created |
| P1 | Telemetry data collection and learning | Active-user, Pipeline performance, and Failure distribution metrics already exist. Improve data quality, long-term samples, dashboards, and release comparisons. | To be created |
| P1 | Capture guidance improvements | Extend existing capture advice and image-order confirmation with examples of orbit paths, speed, overlap, and lighting. | To be created |
| P1 | Panorama platform and large-scene acceptance | Main supports standard ERP video and images. Continue checking source diversity, long inputs, seams, directional coverage, and large-scale densification. The 10-million cap is permitted, not a verified achieved count. | [Verification record](docs/PANORAMA_IMAGES_V1.md) |
| P2 | AI masking and fine image matting Beta | Validate native person-exclusion and subject-Alpha inference, corrections, model distribution, and COLMAP/Brush semantics before enabling verified platforms. Not available on current main. | To be created |
| P2 | Mesh export | Convert reconstruction results to common Mesh formats with documented texture, coordinate-system, and quality options. | To be created |
| Under validation | Reshoot v2 | Multiple cameras, mixed sizes, warm-start training and multi-batch ERP reshoots. The native Ballintoy core flow passed; real cross-device inputs and desktop interaction still need acceptance. | [PR #69](https://github.com/ooolabdev/ooosplat/pull/69) · [Validation notes](docs/RESHOOT_V2.md) |
| P3 | Panorama input expansion | Explore additional projections; dual-fisheye, stereo, VR180, cropped panoramas, and private originals remain unsupported. | To be created |

## Completed

“Implemented · main” means integrated into the main source tree, not necessarily released or accepted on all platforms. See linked records for real-media, hardware, and platform limitations.

| Status | Item | Delivery | GitHub Issue / PR |
| --- | --- | --- | --- |
| Completed | Ubuntu 24.04 Alpha | Provides an x86_64 `.deb` desktop package and CLI using system FFmpeg/FFprobe and bundled pinned COLMAP/Brush runtimes. COLMAP uses CUDA or CPU as supported. | [#5](https://github.com/ooolabdev/ooosplat/issues/5) / [PR #10](https://github.com/ooolabdev/ooosplat/pull/10) |
| Completed | Apple Silicon macOS Alpha | Provides an application-bundled FFmpeg, FFprobe, CPU COLMAP, and Brush workflow for macOS 15+ arm64. | [#4](https://github.com/ooolabdev/ooosplat/issues/4) / [PR #8](https://github.com/ooolabdev/ooosplat/pull/8) |
| Completed | Embedded Gaussian preview and animation export | Supports `.ply` loading, camera navigation, whole-model transforms, animation preview, and Gaussian and portrait-video export. | [#3](https://github.com/ooolabdev/ooosplat/issues/3) |
| Completed | Automatic COLMAP CUDA acceleration | Detects NVIDIA drivers and Compute Capability, enables GPU feature extraction and matching when supported, and otherwise falls back to CPU. | [#6](https://github.com/ooolabdev/ooosplat/issues/6) |
| Completed · 0.4.0 | Stage-level pipeline resume and time estimation | Validates and reuses frame, mask, feature, matching, and sparse-reconstruction checkpoints. Missing or damaged stages safely fall back for rerun, with duration estimates. | [PR #27](https://github.com/ooolabdev/ooosplat/pull/27) |
| Completed · 0.4.0 | Improved Brush training progress | Updates the training-stage percentage continuously from actual Brush steps so long-running training remains visible and traceable. | Implemented in 0.4.0 |
| Completed · 0.4.0 | Removed the 50% registration stop threshold | Continues to Brush when valid registered images and 3D points exist. Low registration remains a quality warning but no longer stops automatically below 50%. | Implemented in 0.4.0 |
| Completed · 0.4.0 | Automatic masks for transparent video and images | Detects transparent MOV and PNG media, preserves unpremultiplied source color and Alpha for Brush, and generates strict 0/255 COLMAP masks at the 0.5 Alpha threshold. | [PR #19](https://github.com/ooolabdev/ooosplat/pull/19) and follow-up work |
| Completed · 0.4.0 | Image-sequence input | Unifies video and image input. Image sequences use a shared camera, exhaustive matching, and the incremental Mapper, with automatic masks for transparent PNG files. | [PR #19](https://github.com/ooolabdev/ooosplat/pull/19) |
| Completed · 0.4.0 | Gaussian editing | Supports rectangle, sphere, and box selection, non-destructive deletion, crop freezing, undo/redo, and saving to `edit.ply`, with a foundation compatible with future AI Agent workflows. | Implemented in 0.4.0; issue to be created |
| Completed · 0.4.0 | English UI and Chinese/English switching | Switches instantly between Simplified Chinese and English, chooses the first-run default from the system language, and persists explicit choices across restarts; task, settings, preview, status, and interaction guidance are covered. | Implemented in 0.4.0; issue to be created |
| Completed · 0.5.0 | Quality v2 automatic parameters and video bridge-frame recovery | Enables optional experimental Auto Optimize by default, planning video sampling and COLMAP limits by preset. When initial reconstruction coverage is low, it adds bridge frames within budget and safely returns to the original model if recovery fails. | Implemented in 0.5.0 |
| Completed · 0.5.0 | VRAM-aware Brush training and OOM downgrade | Detailed quality selects resolution, densification, and Splat limits from VRAM, with at most one lower-profile retry after an explicit out-of-memory failure. | Implemented in 0.5.0 |
| Completed · 0.5.0 | Same-camera incremental high-resolution reshoots | Completed projects can add video or images captured with the same device, lens, and resolution. OOOSplat reuses the source database, shared camera, and sparse model, processes only new views, fully retrains Brush, and supports transparent MOV/PNG media. | [PR #49](https://github.com/ooolabdev/ooosplat/pull/49) |
| Completed · 0.5.0 | Windows Brush GPU stability and device-loss diagnostics | Stabilizes Brush device selection on systems with one NVIDIA discrete GPU, separates out-of-memory and device-loss failures, and provides clearer recovery guidance. | Implemented in 0.5.0 |
| Completed · 0.5.0 | Anonymous planner effectiveness telemetry and preview fixes | Records planning, reconstruction, training, and result metrics without collecting media, paths, or logs, while improving preview ground-grid resource handling and large Transform scales. | Implemented in 0.5.0 |
| Completed · 0.6.0 | Local MCP v1 and shared task service | Provides seven token-free, loopback-only MCP tools for app inspection, task creation and start, task listing and status, incremental log reading, and run-specific cancellation. The GUI and local Agents share task state, the execution lock, progress, and results. | Implemented in the 0.6.0 main development version |
| Completed · 0.6.1 | Update notifications | Checks platform-compatible releases, shows dates and notes, supports copying download links and revisiting updates in Settings; installation remains manual. | [Feature notes](docs/update-notifications.md) |
| Completed · 0.6.1 | Scene gizmos and edit saving | Adds translation axes, center-pivot rotation rings, inversion shortcuts, and saved views; separates edit saving from model export and improves preview sorting. | Implemented in 0.6.1 |
| Implemented · main | External Gaussian and COLMAP project import | Imports standard/compressed PLY, SOG, SPZ, SPLAT, and KSPLAT from Completed; detects COLMAP, keeps external data read-only, and enables eligible reshoots. | Implemented on main |
| Implemented · main | Multi-format model, landscape-video, and HTML export | Select PLY, compressed PLY, SPZ v4, or bundled SOG directly without an intermediate PLY; supports portrait/landscape video, offline HTML, cancellation, and safe publication. | [Export verification](docs/model-export-verification.md) |
| Implemented · main | Panorama video and image input | Reconstructs standard full ERP spherically, then generates six-face training caches for registered images. Dedicated VRAM budgets and recipes retain source media, time/source mappings, and stage reuse. | [Video notes](docs/PANORAMA_VIDEO_V1.md) / [Image notes](docs/PANORAMA_IMAGES_V1.md) |
| Implemented · main | Fast input detection and image-order confirmation | Lightweight ordinary/panorama detection is reused across quality changes. More than 100 images require order confirmation; sequential/exhaustive choices persist for resume, and the queue waits for answers. | Implemented on main |
| Implemented · main | Preview navigation and camera fixes | Fixes post-zoom panning scale, orthographic grid clipping, and fixed-view orbit jumps. Adds horizontal WASD at 15% of the original speed and first-training-view locating. | [Navigation and verification](docs/PREVIEW_NAVIGATION.md) |
| Implemented · main | Multi-pass progress and runtime monitoring | Isolates operations and late events, retains monitor state, separates registration counts, face preparation, and actual training iterations, and reports matching activity and silent periods. | Implemented on main |
| Implemented · main | Temporary PNG/XMP focal workaround | Correctly parses rational XMP values and overrides initialization only for eligible ordinary PNGs with the confirmed affected runtime. Image bytes stay unchanged; incompatible caches are invalidated. | [#73](https://github.com/ooolabdev/ooosplat/issues/73) / [Scope and removal](docs/temporary-png-focal-workaround.md) |

## Tracking and Contributions

The actual feature scope, technical discussion, and implementation status are governed by the linked GitHub Issues. Contributions, use cases, and technical feedback are welcome in the corresponding issue.

Items marked “To be created” do not yet have a dedicated issue. Once one exists, this page should be updated with its permanent issue number and link. Priorities may change as requirements and implementation constraints evolve; a priority change does not mean a feature has been cancelled.
