# Auto Optimize parameter configuration

> New runs use the [generation defaults and retry policy](GENERATION_POLICY.md). Legacy VRAM tiers, optimization-off recipes and profile downgrades below remain for historical snapshot compatibility. New Detailed runs use source resolution and the [effective-pixel budget (k=0.5)](FINE_SPLAT_BUDGET.md) for ordinary media; panoramas retain 40× sparse points. Automatic retry authorizes point reductions or reshoot-plan fallbacks only. Fast/Balanced retain their optimized recipes.

Reconstruction and training presets are defined in [`config/pipeline-optimization.json`](../config/pipeline-optimization.json). The configuration is embedded at build time. Developers can set `OOOSPLAT_PIPELINE_CONFIG` to an external JSON file before starting the app; restart after changes. Missing, malformed, unknown, or unsafe settings are rejected as a whole, with a message explaining the fallback to embedded defaults. `schemaVersion` remains `1`.

```powershell
$env:OOOSPLAT_PIPELINE_CONFIG = "E:\configs\pipeline-optimization.json"
npm run dev:local
```

## Auto Optimize off

Panorama videos and images use the separate optional `panoramaTraining` configuration, not the ordinary Brush presets below. `maxSplatsWithoutOptimization` defaults to 1M / 2M / 3M and applies only with Auto Optimize off. `maxSplats` retains the 1M / 5M / 10M tier ceilings with optimization on, further limited by quality-specific VRAM budgets. Iterations remain 12k / 30k / 45k. See [panorama recipes and validation](PANORAMA_IMAGES_V1.md#全景训练配方-v1) for the full budget table, initial-point overrides and one-shot OOM retries. Older external configurations missing the panorama section or the new off-mode fields use the corresponding embedded defaults; saved task recipes are not rewritten.

Videos use the original fixed-ratio sampling. Initial reconstruction and reshoots share the preset's SFM limits. Candidate pools, Bridge, staged resolutions, loop detection, Caspar BA, and automatic High OOM retries are disabled. Image sequences retain all valid images and use exhaustive matching.

| Preset | Video retention | SFM long edge / features per image | Brush steps | Brush resolution | Growth threshold / fraction / stop |
| --- | ---: | ---: | ---: | ---: | --- |
| Fast | 30% | 1200 / 8192 | 8,000 | 1200 | 0.0025 / 0.10 / 15,000 |
| Balanced | 50% | 1600 / 8192 | 15,000 | 1600 | 0.0025 / 0.10 / 15,000 |
| High | 100% | 2000 / 8192 | 30,000 | 2000 | 0.0025 / 0.10 / 15,000 |

These SFM limits do not rewrite original image files. The older `incrementalSfmMaxImageSize` and `incrementalSfmMaxFeatures` names remain accepted as aliases. The default Brush refine interval is 200. Fast ends before the growth-stop step, Balanced ends at it, and High continues optimizing afterward. These settings do not disable Brush's other refine behavior or add an application-level splat cap; the locked Brush runtime retains its own 10,000,000 cap. Old configurations without densification overrides remain readable; resuming saved tasks preserves their recorded training configuration.

The compatible 0.10 growth fraction retains the new gradient threshold. Old and new gradient measurements differ, so the old 0.00004 threshold is not interchangeable. A 2026-10-06 RTX 3060 Ti Fast toy comparison, with the same reconstruction, 8,000 steps, 1,200 resolution and 19 evaluation views, reduced splats from 533,775 to 316,951 and training time by approximately 28.27% when the fraction changed from 0.25 to 0.10. Rendering was slightly closer to the old runtime, with a small decline in foreground metrics against source photos. This evidence covers one Fast scene, not equivalent hardware quality validation of Balanced and High.

## Auto Optimize on

| Profile | Initial / rescue FPS | Working long edge | SFM long edge / features | Two-view tracks | Brush steps / resolution |
| --- | ---: | ---: | ---: | --- | ---: |
| Fast | 6 / 9 | 1600 | 1200 / 4096 | Off | 8,000 / 1600 |
| Balanced | 8 / 12 | 1920 | 1600 / 8192 | Off | 15,000 / 1920 |
| High Low | 12 / 15 | 3200 | 3200 / 16384 | On | 30,000 / 3200 |
| High Standard | 12 / 15 | 3840 | 3200 / 16384 | On | 30,000 / 3840 |
| High Large | 12 / 15 | Source long edge | 3200 / 16384 | On | 30,000 / source long edge |

High starts with Low when memory is unknown or below 8192 MiB, Standard at 8192–12287 MiB, and Large at 12288 MiB or above.

| Profile | Growth threshold | Fraction | Growth-stop step | Configured splat cap | OOM fallback |
| --- | ---: | ---: | ---: | ---: | --- |
| Fast | 0.0022 | 0.11 | 7,000 | 500,000 | None |
| Balanced | 0.0020 | 0.12 | 13,000 | 1,000,000 | None |
| High Low | 0.0018 | 0.13 | 21,000 | 1,200,000 | High Emergency |
| High Standard | 0.0016 | 0.14 | 23,000 | 1,500,000 | High Low |
| High Large | 0.0015 | 0.15 | 24,000 | 4,000,000 | High Standard |
| High Emergency | 0.0023 | 0.10 | 16,000 | 1,000,000 | None |

The runtime cap never falls below the initial SFM point count. Fast and Balanced have no automatic OOM fallback. High retries at most once per task, only for an explicitly classified out-of-memory error; device loss and other process failures do not trigger this retry.

## Video loop detection

`automaticOptimizationOn.loopDetection` defaults to `true` in Fast, Balanced, and High. Sequential video matching additionally searches for visually similar, distant viewpoints, helping reconnect a capture that returns to its starting location. Matching can take longer, and suitable overlap is not guaranteed.

- The bundled `lib/colmap/vocab_tree_faiss_flickr100K_words256K.bin` is checked against its locked size and SHA256 before matching. No network download or additional installer resource is needed. Missing or damaged data produces an actionable error, not silent fallback.
- The command explicitly uses `--SequentialMatching.loop_detection 1` and an absolute vocabulary path, retaining overlap 10, detection period 10, and 50 retrieval candidates. CPU/GPU SIFT matching and GPU selection are unchanged.
- Auto Optimize off explicitly passes `loop_detection=0`. Image sequences and reshoots still use exhaustive matching; Bridge still matches its specified image pairs.
- Missing fields in old configuration snapshots default to true, gated by the task's saved optimization toggle. Completed matching checkpoints are not automatically recomputed; unfinished matching resumes with the task's saved configuration.

## Bridge and Caspar

- Bridge applies only to optimized videos when initial registration falls below `bridgeTriggerRatio` (default 0.8) and the candidate budget permits it. It fills the largest internal gap first, then endpoint gaps. Failed backfill rolls back to the initial model. Images do not use Bridge. Registration below `goodRegistrationRatio` (also 0.8) warns about quality without blocking training.
- SIFT GPU availability and Caspar BA are independent. Caspar requires Auto Optimize, compatible hardware and drivers, a successful real GPU probe, and both keypoint and verified-match totals reaching their thresholds (2,000,000 each by default). It keeps local BA on Ceres and uses Caspar only globally. Otherwise all BA uses Ceres; macOS always uses CPU Ceres.
- A failed Caspar Mapper attempt clears its output and retries once with local/global Ceres, disabling Caspar for that engine/GPU combination for this app session.

The current Caspar policy is based on the [toy comparison](caspar-toy-benchmark-report-20261004.md) and [001 local-BA investigation](caspar-001-local-report-20261004.md). On the tested RTX 3060 Ti datasets, Caspar was slower on the small toy case; with global Caspar fixed, local Caspar increased total Mapper time by approximately 19%–31%. These results cover specific datasets and solver defaults, not all GPUs or final Brush rendering quality. The dual thresholds require continued validation.

## Configuration boundaries

Validation rejects unknown fields, nonpositive dimensions or steps, invalid ratios, rescue FPS below initial FPS, cyclic or unbounded OOM chains, and incomplete High profiles. The loop-detection switch is configurable, but matcher selection, incremental Mapper, deterministic Bridge selection, Ceres fallback, and the one-retry OOM limit remain fixed in code to preserve project and checkpoint semantics.
