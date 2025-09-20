SHELL := /bin/sh

# Firmware configuration (release-only)
# Defaults can be overridden via environment variables on invocation.
CHIP        ?= STM32L051C8Tx
TARGET_TRIPLE ?= thumbv6m-none-eabi
BIN_NAME    ?= smart-battery

# Probe address default (hardcoded) — can be overridden by env
# Example override: `PROBE_ADDR=xxxx make attach`
PROBE_ADDR  ?= 0d28:0204:2BDC77EE006DCE9B589D7AD8F22BD989

# Resolve workspace root and target dir reliably even when invoked from elsewhere
THIS_MAKEFILE := $(abspath $(lastword $(MAKEFILE_LIST)))
CRATE_DIR     := $(abspath $(dir $(THIS_MAKEFILE)))
WORKSPACE_ROOT:= $(abspath $(CRATE_DIR)/../..)

# Respect CARGO_TARGET_DIR if provided; otherwise default to workspace target
ifeq ($(strip $(CARGO_TARGET_DIR)),)
TARGET_DIR := $(WORKSPACE_ROOT)/target
else
TARGET_DIR := $(abspath $(CARGO_TARGET_DIR))
endif

RELEASE_DIR := $(TARGET_DIR)/$(TARGET_TRIPLE)/release
ELF         := $(RELEASE_DIR)/$(BIN_NAME)

CARGO := cargo
PROBE := probe-rs

.PHONY: all build run attach reset reset-attach clean

all: build

# Build release firmware every time (do not rely on ELF timestamp)
build:
	$(CARGO) build --release --manifest-path $(CRATE_DIR)/Cargo.toml

# Flash and run (prints defmt logs via probe-rs)
run: build
	$(PROBE) run --chip $(CHIP) --probe $(PROBE_ADDR) $(ELF)

# Attach to the running target with release ELF symbols loaded
attach: build
	$(PROBE) attach --chip $(CHIP) --probe $(PROBE_ADDR) $(ELF)

# Reset the target MCU
reset:
	$(PROBE) reset --chip $(CHIP) --probe $(PROBE_ADDR)

# Reset then attach in one go
reset-attach: reset attach

# Clean build artifacts for this crate
clean:
	$(CARGO) clean -p $(BIN_NAME) --manifest-path $(CRATE_DIR)/Cargo.toml
