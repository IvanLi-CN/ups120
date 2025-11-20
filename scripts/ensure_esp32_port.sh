#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
CACHE_FILE="$REPO_ROOT/.esp32-port"

err() { printf '%s\n' "$1" >&2; }

if ! command -v espflash >/dev/null 2>&1; then
  err "[esp32-port] espflash not found; install via 'cargo install espflash'"
  exit 127
fi

# espflash list-ports prepends spaces before device names; allow leading whitespace
PORT_LIST=$(espflash list-ports 2>/dev/null | awk '/^[[:space:]]*\/dev\// {print $1}')
if [ -z "$PORT_LIST" ]; then
  err "[esp32-port] 未检测到任何 ESP32 串口；请插入板子或通过 PORT=/dev/... 指定。"
  exit 1
fi

ALL_PORTS=()
while IFS= read -r line; do
  [ -n "$line" ] && ALL_PORTS+=("$line")
done <<<"$PORT_LIST"

CU_LIST=$(printf '%s\n' "${ALL_PORTS[@]}" 2>/dev/null | grep '^/dev/cu\.' || true)
CU_PORTS=()
while IFS= read -r line; do
  [ -n "$line" ] && CU_PORTS+=("$line")
done <<<"$CU_LIST"

contains_port() {
  local needle="$1"
  shift || true
  local entry
  for entry in "$@"; do
    if [ "$entry" = "$needle" ]; then
      return 0
    fi
  done
  return 1
}

pick_port() {
  local candidate="$1"
  echo "$candidate" > "$CACHE_FILE"
  echo "$candidate"
}

if [ -n "${PORT:-}" ]; then
  if contains_port "$PORT" "${ALL_PORTS[@]}"; then
    pick_port "$PORT"
    exit 0
  fi
  err "[esp32-port] 指定的 PORT=$PORT 当前不可用；有效端口: ${ALL_PORTS[*]}"
  exit 1
fi

if [ "${#CU_PORTS[@]}" -eq 1 ]; then
  pick_port "${CU_PORTS[0]}"
  exit 0
fi

if [ "${#ALL_PORTS[@]}" -eq 1 ]; then
  pick_port "${ALL_PORTS[0]}"
  exit 0
fi

if [ -f "$CACHE_FILE" ]; then
  cached=$(cat "$CACHE_FILE" 2>/dev/null || true)
  if [ -n "$cached" ] && contains_port "$cached" "${ALL_PORTS[@]}"; then
    echo "$cached"
    exit 0
  fi
fi

err "[esp32-port] 检测到多个串口: ${ALL_PORTS[*]}"
err "[esp32-port] 请使用 PORT=/dev/cu.xxx make ups-run（或写入 .esp32-port）来选择。"
exit 1
