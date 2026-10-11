# CODING_STANDARDS.md

Read during code review, not implementation. Rules here are enforced by the
reviewer agent against the diff under review.

## Python (engines/*/sidecar)

- **Threshold pre-sigmoid logits at `> 0`, never convert float tensors with
  `.bool()`.** Torch semantics map every nonzero float to `True`; logits are
  ±10..±20 in practice and never exactly 0, so `.bool()` yields full-frame
  foreground. This exact bug shipped in issue #1. Flag any `.bool()` /
  `!= 0` conversion on model output, and any `> 0.5` threshold applied to
  logits (that constant belongs to probabilities, post-sigmoid).
- **Sort imports with ruff** (`ruff check` + `ruff format` gate CI and
  pre-review): `uv run ruff check . && uv run ruff format --check .`
  from the sidecar directory.

## Reviewer checklist (per sidecar diff)

- [ ] `uv run pytest -q` passes in the touched sidecar directory (no GPU or
      weights needed; real-backend tests are numpy-stubbed).
- [ ] No float→bool conversion on model outputs (see rule above).
- [ ] New Python deps go in `[dependency-groups].dev` of the sidecar's
      `pyproject.toml`, never a global env change.
- [ ] Derived assets keep the non-destructive rule: `final.ply` is never
      overwritten; splat/mesh products are derived files only.
