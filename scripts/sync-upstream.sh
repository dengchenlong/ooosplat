#!/usr/bin/env bash
# sync-upstream.sh — 每日同步：从原作者仓库 (upstream) 拉取更新并推送到自己的 fork (origin)
# 用法: bash scripts/sync-upstream.sh [--timeout SECONDS]   （在仓库内任意位置均可）
#
# 每一步都带硬超时并立即回显。2026-11-02 那次「脚本卡死 180s/420s、连 [1/4] 都没打印」，
# 表现是整段输出被管道缓冲 + git 没有超时：既看不到进度，也永远等不到结束。
set -uo pipefail

STEP_TIMEOUT=180
if [ "${1:-}" = "--timeout" ]; then
  STEP_TIMEOUT="${2:?--timeout needs a value}"
fi

say() { printf '%s\n' "$*"; }

# run_step <描述> <命令...>：带超时执行，超时时明确报出是哪一步。
run_step() {
  local what="$1"; shift
  say "==> ${what} (timeout ${STEP_TIMEOUT}s)"
  local rc=0
  if command -v timeout >/dev/null 2>&1; then
    timeout "$STEP_TIMEOUT" "$@" || rc=$?
  else
    "$@" || rc=$?
  fi
  if [ $rc -eq 124 ]; then
    say "错误：${what} 超过 ${STEP_TIMEOUT}s 未完成（超时被杀）" >&2
  fi
  return $rc
}

# 定位仓库根目录（脚本位于 <repo>/scripts/ 下）
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
say "仓库根目录：${ROOT}"

if ! command -v git >/dev/null 2>&1; then
  say "错误：找不到 git（PATH 里没有）" >&2
  exit 127
fi

run_step "[1/4] 拉取 upstream" git fetch upstream || exit $?

echo "==> [2/4] 检查本地工作区是否干净 ..."
# 检查工作区是否干净；忽略本脚本自身（脚本仅本地存放、不提交）
if [ -n "$(git status --porcelain | grep -v -F 'scripts/sync-upstream.sh' || true)" ]; then
  echo "错误：本地有未提交的改动，请先 commit 或 stash 后再同步：" >&2
  git status --short >&2
  exit 1
fi

echo "==> [3/4] 合并 upstream/main（快进优先）..."
if ! run_step "合并 upstream/main" git merge --ff-only upstream/main; then
  git merge --abort 2>/dev/null || true
  cat >&2 <<'EOF'
错误：无法快进合并 upstream/main（可能存在分叉或冲突）。
请手动处理：
  1. git log --oneline main..upstream/main   # 查看上游新提交
  2. git log --oneline upstream/main..main   # 查看本地独有提交
  3. 手动 git merge upstream/main 解决冲突后再推送
EOF
  exit 1
fi

echo "==> [4/4] 推送到 origin (fork) ..."
if ! run_step "[4/4] 推送到 origin (fork)" git push origin main; then
  echo "错误：推送 origin 失败（网络或权限）；本地已合并但 fork 未更新" >&2
  exit 1
fi

echo "---- 同步结果摘要 ----"
echo "本地 main      : $(git rev-parse --short HEAD)  $(git log -1 --format=%s HEAD)"
echo "upstream/main  : $(git rev-parse --short upstream/main)"
echo "origin/main    : $(git rev-parse --short origin/main)"
if [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] \
   && git merge-base --is-ancestor upstream/main HEAD; then
  echo "状态：本地 main 已包含 upstream/main 全部提交，并已推送到 origin ✔"
else
  echo "状态：同步不完整——本地 main 缺少 upstream 提交或未推送到 origin ✘" >&2
  exit 1
fi
