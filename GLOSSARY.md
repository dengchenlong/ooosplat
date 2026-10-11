# GLOSSARY.md

Single-source domain vocabulary for OOOSplat. One entry per term; when a term
gains a second meaning, split or disambiguate it here (see `docs/agents/domain.md`).

## Pipeline & stages

- **PipelineStage**: Enum of the splat pipeline's ordered stages. The Segment
  stage (issue #3) slots in before training; stages carry checkpoints so a run
  can resume where it stopped.
- **断点续跑 (resume-from-checkpoint)**: Property of pipeline stages: after an
  interrupt, a stage re-enters from its persisted `state.json` checkpoint
  rather than restarting. Checked per-frame/atomic.

## Segmentation

- **SAM2 sidecar**: Python subprocess (`engines/sam2/sidecar/`) speaking a
  JSON-lines protocol over stdio. Owns all SAM2 inference; the TS main process
  never imports ML code. Local-only: weights on disk, no cloud API.
- **prompt**: First-frame selection passed to the sidecar: point(s) and/or box
  coordinates selecting the object to track.
- **propagate**: SAM2 video-predictor pass spreading the prompt's mask across
  all frames. Emits per-frame `mask` responses then `propagate_done`.
- **anchor (锚点)**: A re-prompt added mid-video to fight loop drift; plan is
  one per ~90° of view change.
- **logit mask**: SAM2 video predictor's raw output: float32, pre-sigmoid,
  typically ±10..±20. Convert to a binary mask with `> 0`, never `.bool()`
  (see CODING_STANDARDS.md).

## Data & assets

- **final.ply**: The trained splat scene. **Never overwritten or modified** —
  all splits/edits derive from it (same philosophy as `edit.ply`).
- **OOSMASK**: Instance bitmap over the splat's points marking object vs
  background; produced by B3 back-projection voting (issue #4).
- **object.ply / background.ply**: Derived point-cloud splits of `final.ply`
  by OOSMASK. Regenerable at any time.
- **object.glb**: Vertex-colored mesh of the dynamic object (stage 1, issue
  #10): per-frame depth → TSDF → marching cubes.
- **masked dataset**: Brush training dataset built from frames + per-frame
  masks; object and background datasets come in pairs (`--invert-masks`).

## Voting (B3)

- **back-projection voting (反投影投票)**: Project each per-frame mask through
  COLMAP cameras onto the splat points; a point's vote count decides
  object/background. Footprint dilation + multi-frame voting + DBSCAN cleanup
  counter occlusion.

## Runtime

- **engines/manifest.json**: Registry of downloadable engines/sidecars;
  `scripts/verify-engines.ps1` (or `npm run verify:engines`) validates the
  install. Model weights land on D: (`D:\ooosplat-engines\`), never C:.
