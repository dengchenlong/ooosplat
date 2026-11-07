#!/usr/bin/env bash
# Installs the repo's git hooks (hooks are not versioned, this is the installer).
# Run once per clone: bash scripts/install-hooks.sh
#
# Note: git is a native Windows binary here, so it must not be handed an MSYS
# path (/g/...) — cd into the repo and let git read its own cwd instead.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOOKS="$(git rev-parse --git-common-dir)"
case "$HOOKS" in
  # Absolute POSIX path (/g/ooosplat/.git), or an absolute Windows path with a
  # drive letter (G:/ooosplat/.git). The drive-letter form is what git returns
  # inside a worktree on this host; treating it as relative used to prepend
  # $ROOT and create a bogus "$ROOT/G:/..." directory tree instead of installing.
  /*|[A-Za-z]:/*) ;;
  *) HOOKS="$ROOT/${HOOKS#./}" ;;
esac

mkdir -p "$HOOKS/hooks"
cat >"$HOOKS/hooks/pre-push" <<'EOF'
#!/usr/bin/env bash
# Delegates to the versioned script so the guardrail lives in the repo.
cd "$(git rev-parse --show-toplevel)" || exit 1
exec ./scripts/pre-push.sh "$@"
EOF
chmod +x "$HOOKS/hooks/pre-push"

echo "installed $HOOKS/hooks/pre-push"
echo "bypass for one push: git push --no-verify"