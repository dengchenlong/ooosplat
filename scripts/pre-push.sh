#!/usr/bin/env bash
# pre-push guardrail: the same gates the CI workflows run, so a push cannot land
# unformatted / clippy-dirty / failing Rust changes while GitHub Actions is
# unavailable on this fork.
#
# Installed by `scripts/install-hooks.sh` as .git/hooks/pre-push (hooks live in
# the common git dir, so every worktree shares it).
#
# Scope: only runs when src-tauri/**/*.rs changed since the remote ref; a docs
# or notes push costs nothing. Bypass with `git push --no-verify` when you have
# already run the gates by hand.
set -uo pipefail

ZERO="0000000000000000000000000000000000000000"
check=0
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
  if git diff --name-only "$base" "$local_sha" 2>/dev/null | grep -q '^src-tauri/.*\.rs$'; then
    check=1
  fi
done

if [ "$check" -eq 0 ]; then
  exit 0
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
TAURI_DIR="$REPO_ROOT/src-tauri"
if [ ! -d "$TAURI_DIR" ]; then
  exit 0
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

echo "[pre-push] ok"
