#!/usr/bin/env bash
set -euo pipefail

# Ensure a unique STM32 debug probe selector is available.
# Prints selector to stdout and caches in .stm32-port.
# Resolution order:
# 1) PROBE env (if present in probe-rs list)
# 2) PORT env alias (if present)
# 3) cached .stm32-port if still connected
# 4) single ST-Link present
# 5) single probe present
# 6) interactive selection (scripts/select_stm32_probe.sh)

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
CACHE_FILE="$REPO_ROOT/.stm32-port"

if ! command -v probe-rs >/dev/null 2>&1; then
  echo "[error] probe-rs not found; install via 'cargo install probe-rs'" >&2
  exit 127
fi

ALL_OUTPUT=$(probe-rs list || true)

index_lines() {
  echo "$ALL_OUTPUT" | grep -E '^[[:space:]]*[0-9]+:|^\[[0-9]+\]:'
}

tokens() { echo "$ALL_OUTPUT" | sed -n -E 's/.*-- ([^ ]+).*/\1/p'; }
has_token() { tokens | grep -Fxq "$1"; }

pick_and_cache() {
  local sel="$1"
  echo "$sel" > "$CACHE_FILE"
  echo "$sel"
}

if [ -n "${PROBE:-}" ] && has_token "$PROBE"; then
  pick_and_cache "$PROBE"
  exit 0
fi

if [ -n "${PORT:-}" ] && has_token "$PORT"; then
  pick_and_cache "$PORT"
  exit 0
fi

read_cache() {
  local file="$1"
  [ -f "$file" ] || return 1
  cat "$file" 2>/dev/null || true
}

if cached=$(read_cache "$CACHE_FILE"); then
  if [ -n "$cached" ] && has_token "$cached"; then
    echo "$cached"
    exit 0
  fi
fi

ST_LINES=$(index_lines | grep -Ei 'ST[- ]?Link|0483:3748' || true)
COUNT_ST=$(echo "$ST_LINES" | grep -E '^[[:space:]]*[0-9]+:|^\[[0-9]+\]:' | wc -l | tr -d ' ')
if [ "$COUNT_ST" = "1" ]; then
  sel=$(echo "$ST_LINES" | sed -n 's/.*-- \([^ ]\+\).*/\1/p' | head -n1)
  if [ -n "$sel" ]; then
    pick_and_cache "$sel"
    exit 0
  fi
fi

COUNT_ALL=$(index_lines | wc -l | tr -d ' ')
if [ "$COUNT_ALL" = "1" ]; then
  sel=$(index_lines | sed -n 's/.*-- \([^ ]\+\).*/\1/p')
  if [ -n "$sel" ]; then
    pick_and_cache "$sel"
    exit 0
  fi
fi

exec "$SCRIPT_DIR/select_stm32_probe.sh"
