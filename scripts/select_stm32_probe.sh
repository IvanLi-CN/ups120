#!/usr/bin/env bash
set -euo pipefail

# Interactive selector for probe-rs debug probe. Outputs selected selector and
# optionally runs a provided make target.
#
# Usage:
#   scripts/select_stm32_probe.sh [make-target [extra make VARS...]]

if ! command -v probe-rs >/dev/null 2>&1; then
  echo "[error] probe-rs not found; install via 'cargo install probe-rs'" >&2
  exit 127
fi

LINES=$(probe-rs list | grep -E '^[[:space:]]*[0-9]+:|^\[[0-9]+\]:')

if [ -z "$LINES" ]; then
  echo "[error] no debug probes found" >&2
  exit 1
fi

echo "Available debug probes:" >&2
echo "$LINES" | sed 's/^/  /' >&2

read -rp "Select index: " IDX

SEL_LINE=$(echo "$LINES" | grep -E "^[[:space:]]*$IDX:|^\[$IDX\]:" | head -n1 || true)

if [ -z "$SEL_LINE" ]; then
  echo "[error] invalid selection: $IDX" >&2
  exit 2
fi

SELECTOR=$(echo "$SEL_LINE" | sed -E 's/.*-- ([^ ]+).*/\1/')

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
echo "$SELECTOR" > "$REPO_ROOT/.stm32-port"

if [ $# -gt 0 ]; then
  TARGET="$1"; shift || true
  echo "[select] PROBE=$SELECTOR -> make $TARGET $*" >&2
  make $TARGET PROBE="$SELECTOR" "$@"
else
  echo "$SELECTOR"
fi
