#!/usr/bin/env bash
# pre-push guardrail: the same gates the CI workflows run, so a push cannot land
# unformatted / clippy-dirty Rust or type-broken TypeScript while GitHub Actions
# is unavailable on this fork.
#
# Installed by `scripts/install-hooks.sh` as .git/hooks/pre-push (hooks live in
# the common git dir, so every worktree shares it).
#
# Scope is per file type, so an unrelated push stays cheap:
#   src-tauri/**/*.rs          -> cargo fmt --check, cargo clippy -D warnings
#   *.ts / *.tsx / tsconfig*   -> tsc -b
# A docs or notes push costs nothing. Bypass with `git push --no-verify` once you
# have run the gates by hand.
#
# Why tsc is here: `tsc -b` is the only static check this repo has on the
# frontend (there is no eslint/prettier), and it lives inside `npm run build` —
# which only runs in CI. With CI frozen on this fork, two real type errors
# reached the branch unnoticed (fixed in 29f9ec7). `npm test` (vitest, ~43s) is
# deliberately not here; it is a per-ticket gate in `scripts/ci-local.sh`.
set -uo pipefail

ZERO="0000000000000000000000000000000000000000"
need_rust=0
need_ts=0

while read -r local_ref local_sha remote_ref remote_sha; do
  [ "$local_sha" = "$ZERO" ] && continue          # branch deletion
  if [ "$remote_sha" = "$ZERO" ]; then
    # New remote branch: compare against where it forked off a known base, and
    # fall back to the root commit so the comparison is always a resolvable rev.
    base=""
    for candidate in origin/main upstream/main; do
      if base="$(git merge-base "$local_sha" "$candidate" 2>/dev/null)" && [ -n "$base" ]; then
        break
      fi
      base=""
    done
    if [ -z "$base" ]; then
      base="$(git rev-list --max-parents=0 "$local_sha" 2>/dev/null | tail -1)"
    fi
  else
    base="$remote_sha"
  fi
  [ -z "$base" ] && continue

  changed="$(git diff --name-only "$base" "$local_sha" 2>/dev/null)"
  printf '%s\n' "$changed" | grep -q '^src-tauri/.*\.rs$' && need_rust=1
  printf '%s\n' "$changed" \
    | grep -qE '\.(ts|tsx|mts|cts)$|(^|/)tsconfig[^/]*\.json$' && need_ts=1
done

if [ "$need_rust" -eq 0 ] && [ "$need_ts" -eq 0 ]; then
  exit 0
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT" || exit 1

# --- TypeScript -----------------------------------------------------------------
if [ "$need_ts" -eq 1 ]; then
  TSC="$REPO_ROOT/node_modules/typescript/bin/tsc"
  if [ ! -f "$TSC" ]; then
    # Do not pass silently: "hook ran but checked nothing" is worse than no hook.
    echo "[pre-push] TypeScript 改动，但 node_modules/typescript 不存在。" >&2
    echo "[pre-push] 先 cd \"$REPO_ROOT\" && npm ci，或用 git push --no-verify 跳过。" >&2
    exit 1
  fi
  echo "[pre-push] tsc -b"
  if ! node "$TSC" -b; then
    echo "[pre-push] 类型错误（CI 的 npm run build 会红）。本地复现：npx tsc -b" >&2
    exit 1
  fi
fi

# --- Rust -----------------------------------------------------------------------
if [ "$need_rust" -eq 1 ]; then
  TAURI_DIR="$REPO_ROOT/src-tauri"
  if [ ! -d "$TAURI_DIR" ]; then
    echo "[pre-push] 找不到 $TAURI_DIR，跳过 Rust 门禁。" >&2
    exit 1
  fi

  # The Rust toolchain lives on D: on this host; fall back to whatever is on PATH.
  if [ -d "D:/deps/cargo/bin" ]; then
    export RUSTUP_HOME="D:/deps/rustup"
    export CARGO_HOME="D:/deps/cargo"
    case ":$PATH:" in
      *":/d/deps/cargo/bin:"*) ;;
      *) PATH="/d/deps/cargo/bin:$PATH" ;;
    esac
  fi

  # build.rs reads .cache/html-viewer/runtime.js; without it the build fails with a
  # message buried under build-script output.
  if [ ! -f "$REPO_ROOT/.cache/html-viewer/runtime.js" ]; then
    echo "[pre-push] building html-viewer runtime (needed by build.rs)..."
    ( cd "$REPO_ROOT" && node scripts/build-html-viewer.mjs >/dev/null ) || {
      echo "[pre-push] build-html-viewer.mjs failed; fix it or push with --no-verify" >&2
      exit 1
    }
  fi

  cd "$TAURI_DIR" || exit 1

  echo "[pre-push] cargo fmt --check"
  if ! cargo fmt -- --check; then
    echo "[pre-push] 格式不合规。本地修：cd src-tauri && cargo fmt" >&2
    exit 1
  fi

  echo "[pre-push] cargo clippy --all-targets -- -D warnings"
  if ! cargo clippy --all-targets -- -D warnings; then
    echo "[pre-push] clippy 有警告（CI 会红）。修掉再推，或 git push --no-verify" >&2
    exit 1
  fi
fi

echo "[pre-push] ok"
