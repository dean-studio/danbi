#!/usr/bin/env bash
# restart-dev.sh — kill anything holding the Tauri dev port, then start a fresh
# `pnpm tauri dev`. Safe to run repeatedly.
#
# Usage:
#   ./scripts/restart-dev.sh        # foreground
#   ./scripts/restart-dev.sh -b     # background (logs to .dev.log)

set -euo pipefail

PORT="${PORT:-1420}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Prefer rustup toolchain (1.91+) over system rust.
export PATH="$HOME/.cargo/bin:$PATH"

BACKGROUND=0
if [[ "${1:-}" == "-b" || "${1:-}" == "--bg" ]]; then
  BACKGROUND=1
fi

kill_tree() {
  local pid="$1"
  # macOS-safe: kill pid + any known children.
  local kids
  kids=$(pgrep -P "$pid" 2>/dev/null || true)
  for k in $kids; do kill_tree "$k"; done
  kill -9 "$pid" 2>/dev/null || true
}

echo "→ freeing port $PORT"
pids=$(lsof -ti:"$PORT" 2>/dev/null || true)
if [[ -n "$pids" ]]; then
  for p in $pids; do kill_tree "$p"; done
  sleep 0.3
fi

echo "→ killing stray danbi / tauri processes"
pkill -9 -f "target/debug/danbi" 2>/dev/null || true
pkill -9 -f "tauri dev"          2>/dev/null || true
pkill -9 -f "vite"               2>/dev/null || true
sleep 0.2

# Verify port is really free before launching.
if lsof -ti:"$PORT" >/dev/null 2>&1; then
  echo "✗ port $PORT still in use after cleanup" >&2
  lsof -i:"$PORT" >&2 || true
  exit 1
fi

echo "→ starting pnpm tauri dev"
if [[ "$BACKGROUND" -eq 1 ]]; then
  : > .dev.log
  nohup pnpm tauri dev >> .dev.log 2>&1 &
  echo "  pid=$! log=$ROOT/.dev.log"
else
  exec pnpm tauri dev
fi
