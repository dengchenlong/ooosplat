#!/usr/bin/env bash
# ci-local.sh — run the same gates as .github/workflows/{windows,ubuntu}.yml,
# locally, in one command.
#
# Why this exists: GitHub froze Actions on the fork
# ("Workflows are not currently being run on this fork due to the scale of
# GitHub Actions usage"), so a push no longer produces a CI verdict. This is the
# local equivalent, so "it should pass" is never the answer.
#
# Usage, from the repo root:
#   bash scripts/ci-local.sh              # every gate CI runs, minus packaging
#   bash scripts/ci-local.sh --fast       # skip engine download + verify
#   bash scripts/ci-local.sh --full       # also install/verify engines and package
#   bash scripts/ci-local.sh --only rust  # rust | frontend | engines
#
# Exit code is 0 only when every selected gate passed. All gates run even after
# a failure, so one run shows every problem instead of the first.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MODE="default"
ONLY="all"
for arg in "$@"; do
  case "$arg" in
    --fast)   MODE="fast" ;;
    --full)   MODE="full" ;;
    --only)   shift ;;
    --only=*) ONLY="${arg#--only=}" ;;
    rust|frontend|engines) ONLY="$arg" ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

if [[ -t 1 ]] && command -v tput >/dev/null 2>&1 && [[ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]]; then
  B=$(tput bold); D=$(tput dim); R=$(tput sgr0); G=$(tput setaf 2); Y=$(tput setaf 3); RD=$(tput setaf 1)
else
  B=""; D=""; R=""; G=""; Y=""; RD=""
fi

# The Rust toolchain lives on D: on this host; the workflows use `rustup default
# stable` on a clean runner. Fall back to PATH when D: isn't there.
if [[ -d "/d/deps/cargo/bin" ]]; then
  export RUSTUP_HOME="${RUSTUP_HOME:-D:/deps/rustup}"
  export CARGO_HOME="${CARGO_HOME:-D:/deps/cargo}"
  case ":$PATH:" in *":/d/deps/cargo/bin:"*) ;; *) PATH="/d/deps/cargo/bin:$PATH" ;; esac
fi

PASSED=(); FAILED=(); SKIPPED=()

# gate <label> <command...>: run one gate, record the verdict, never abort.
gate() {
  local label="$1"; shift
  printf '\n%s▸ %s%s\n' "$B" "$label" "$R"
  printf '%s  $ %s%s\n' "$D" "$*" "$R"
  local start=$SECONDS
  if "$@"; then
    PASSED+=("$label")
    printf '%s  ✓ pass%s %s(%ss)%s\n' "$G" "$R" "$D" "$((SECONDS - start))" "$R"
  else
    local rc=$?
    FAILED+=("$label")
    printf '%s  ✗ FAIL (exit %s)%s\n' "$RD" "$rc" "$R"
  fi
}

skip() { SKIPPED+=("$1"); printf '\n%s▸ %s%s\n%s  skipped: %s%s\n' "$B" "$1" "$R" "$D" "$2" "$R"; }

want() {
  local group="$1"
  [[ "$ONLY" == "all" || "$ONLY" == "$group" ]]
}

printf '%s%s  ci-local%s  %srepo=%s mode=%s only=%s%s\n' \
  "$B" "$G" "$R" "$D" "$ROOT" "$MODE" "$ONLY" "$R"

# ── toolchain ─────────────────────────────────────────────────────────────
command -v node >/dev/null 2>&1 || { echo "node not on PATH" >&2; exit 127; }
command -v cargo >/dev/null 2>&1 || { echo "cargo not on PATH (D:/deps/cargo/bin)" >&2; exit 127; }
printf '%s  node %s · cargo %s%s\n' "$D" "$(node --version)" "$(cargo --version | awk '{print $2}')" "$R"

# ── frontend ──────────────────────────────────────────────────────────────
if want frontend; then
  # The workflows run `npm ci` on a clean runner. Locally, reinstalling 446MB on
  # every invocation is pure cost, so only do it when node_modules is absent.
  if [[ -d node_modules ]]; then
    skip "npm ci" "node_modules already present (delete it to force a clean install)"
  else
    gate "npm ci" npm ci
  fi
  gate "verify:telemetry:release" npm run verify:telemetry:release
  gate "npm test"                 npm test
  gate "test:engine-scripts"      npm run test:engine-scripts
  gate "verify:colmap-lock"       npm run verify:colmap-lock
  gate "build (tsc -b + vite)"    npm run build
  gate "verify:bundle-resources"  npm run verify:bundle-resources
  gate "verify:licenses"          npm run verify:licenses
fi

# ── rust ──────────────────────────────────────────────────────────────────
if want rust; then
  gate "cargo fmt --check"  cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
  gate "cargo test"         cargo test --manifest-path src-tauri/Cargo.toml
  gate "cargo clippy"       cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
fi

# ── engines (network + ~GB download; CI spends up to 360 min here) ────────
if want engines; then
  if [[ "$MODE" == "full" ]]; then
    gate "setup:engines"  npm run setup:engines
    gate "verify:engines" npm run verify:engines
  else
    skip "setup:engines + verify:engines" "needs a multi-GB download; re-run with --full"
  fi
  if [[ "$MODE" == "full" ]]; then
    gate "package:windows" npm run package:windows
  else
    skip "package:windows" "produces an installer; re-run with --full"
  fi
fi

# ── verdict ───────────────────────────────────────────────────────────────
printf '\n%s%s  ── verdict ──%s\n' "$B" "$G" "$R"
(( ${#PASSED[@]} ))  && printf '  %s✓ %s passed%s: %s\n' "$G" "${#PASSED[@]}" "$R" "${PASSED[*]}"
(( ${#SKIPPED[@]} )) && printf '  %s• %s skipped%s: %s\n' "$Y" "${#SKIPPED[@]}" "$R" "${SKIPPED[*]}"
if (( ${#FAILED[@]} )); then
  printf '  %s✗ %s failed%s: %s\n' "$RD" "${#FAILED[@]}" "$R" "${FAILED[*]}"
  printf '\n%s  CI would be red. Fix the gates above before pushing.%s\n' "$RD" "$R"
  exit 1
fi
printf '\n%s  CI would be green — every selected gate passed.%s\n' "$G" "$R"
