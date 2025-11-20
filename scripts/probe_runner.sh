#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
ENSURE_STM32_PROBE="$SCRIPT_DIR/ensure_stm32_probe.sh"
DEFAULT_PROBE_ADDR=0d28:0204:2BDC77EE006DCE9B589D7AD8F22BD989

# Resolve probe selection if not provided via ENV
if [ -z "${PROBE_ADDR:-}" ] && [ -x "$ENSURE_STM32_PROBE" ]; then
  PROBE_ADDR=$(PROBE="$PROBE" "$ENSURE_STM32_PROBE")
fi

PROBE_ADDR=${PROBE_ADDR:-$DEFAULT_PROBE_ADDR}

ELF="$1"

if [ -z "$ELF" ] || [ ! -f "$ELF" ]; then
  echo "[probe-runner] ELF not found: $ELF" >&2
  exit 2
fi

LOGFMT=${LOGFMT:-"{s}"}

# Keep RTT/defmt output available by using probe-rs run directly.
exec probe-rs run \
  --chip STM32L051C8Tx \
  --probe "$PROBE_ADDR" \
  --log-format "$LOGFMT" \
  "$ELF"
