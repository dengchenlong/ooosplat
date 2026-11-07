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

## Rust (`src-tauri`)

The mechanical rules here are already enforced by `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` in `scripts/pre-push.sh` and
`scripts/ci-local.sh`. What follows are the **judgement calls** those gates
cannot make.

- **A defaulted option must leave existing callers byte-identical.** When
  adding a field to an options struct (e.g. `BrushTrainingOptions::export_name`),
  the default has to reproduce the old behaviour exactly — the old hard-coded
  output name stays the default constant. If an existing caller's bytes change,
  it is a breaking change wearing an option's clothes.
- **Test-only wrappers get `#[cfg(test)]`, not `#[allow(dead_code)]`.** Once
  production code moves to a new entry point, an old wrapper referenced only by
  tests trips `clippy -D warnings`. `allow(dead_code)` silences the real
  regression too; `cfg(test)` keeps the wrapper compiling where it is used and
  keeps the warning honest everywhere else.
- **Async callbacks take the argument by value.** A `FnMut(&T) -> Fut`
  signature cannot unify the lifetime of a borrow with a single `Fut` type
  serving several calls (`lifetime may not live long enough`), and a
  synchronous `FnMut(&T) -> Result<_>` cannot wrap an async callee at all.
  `FnMut(T) -> Fut` with a cheap clone is the shape that works.
- **A stage that cannot be wired into the runner is reported as not done.** Do
  not claim a pipeline stage works because a CLI exercises the same module; say
  which entry point is wired and which is not.
- **Validation of a produced artifact is not the process exit code.** Derived
  assets (PLY, GLB) must be re-parsed and checked (`inspect_gaussian_ply`,
  glTF-Validator) before a run is called successful.

## Reviewer checklist (per `src-tauri` diff)

- [ ] `cd src-tauri && cargo fmt -- --check` and
      `cargo clippy --all-targets -- -D warnings` are clean (the pre-push hook
      runs both; a `--no-verify` push owes them by hand).
- [ ] `cargo test --lib` passes and the count did not drop.
- [ ] No new behaviour changes silently for existing callers (defaults preserve
      old output, old CLI arities still parse).
- [ ] Derived assets only — `final.ply` is still never written.
