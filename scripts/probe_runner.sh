#!/usr/bin/env bash
set -euo pipefail

ELF="$1"
BIN="${ELF}.bin"

if command -v llvm-objcopy >/dev/null 2>&1; then
  OBJCOPY=llvm-objcopy
elif command -v rust-objcopy >/dev/null 2>&1; then
  OBJCOPY=rust-objcopy
else
  echo "Error: llvm-objcopy or rust-objcopy not found. Install with:"
  echo "  rustup component add llvm-tools-preview"
  exit 127
fi

"$OBJCOPY" -O binary "$ELF" "$BIN"

probe-rs download \
  --chip STM32L051C8Tx \
  --probe 0d28:0204:2BDC77EE006DCE9B589D7AD8F22BD989 \
  --binary-format bin \
  --base-address 0x08000000 \
  "$BIN"

# Reset the target to start executing
probe-rs reset \
  --chip STM32L051C8Tx \
  --probe 0d28:0204:2BDC77EE006DCE9B589D7AD8F22BD989
