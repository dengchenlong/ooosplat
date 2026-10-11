# AGENTS.md

## Agent skills

### Issue tracker

Issues are tracked in this repo's GitHub Issues (github.com/dengchenlong/ooosplat), via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` at the repo root + `docs/adr/` for ADRs (created lazily). See `docs/agents/domain.md`.

### Python sidecars

Engine sidecars live under `engines/<name>/sidecar/` (Python, uv-managed). The SAM2 sidecar is at `engines/sam2/sidecar/`: run `uv sync --dev` then `uv run pytest -q` in that directory (tests are numpy-stubbed — no GPU or weights needed); lint with `uv run ruff check .`. Lint + tests gate CI via `.github/workflows/sam2-sidecar.yml`. Review-time rules: `CODING_STANDARDS.md` at the repo root.
