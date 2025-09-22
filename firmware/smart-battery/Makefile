SHELL := /bin/sh

# Firmware configuration (release-only)
# Defaults can be overridden via environment variables on invocation.
CHIP           ?= STM32L051C8Tx
TARGET_TRIPLE  ?= thumbv6m-none-eabi
BIN_NAME       ?= smart-battery

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

# Always pass explicit target to avoid host builds sneaking in
CARGO_TARGET_FLAGS := --target $(TARGET_TRIPLE)

CARGO := cargo
PROBE := probe-rs

.PHONY: all build run attach reset reset-attach clean info run-force

all: build

# Build release firmware every time (do not rely on ELF timestamp)
build:
	@echo "[build] crate=$(CRATE_DIR) target=$(TARGET_TRIPLE)"
	$(CARGO) build --release $(CARGO_TARGET_FLAGS) --manifest-path $(CRATE_DIR)/Cargo.toml
	@echo "[build] ELF=$(ELF)"; \
	if [ -f "$(ELF)" ]; then \
	  stat_cmd=$$(command -v gstat || command -v stat); \
	  if echo $$($$stat_cmd --version 2>/dev/null) | grep -qi 'gnu coreutils'; then \
	    $$stat_cmd -c '[build] mtime=%y size=%s' "$(ELF)"; \
	  else \
	    $$stat_cmd -f '[build] mtime=%Sm size=%z' -t '%Y-%m-%d %H:%M:%S' "$(ELF)"; \
	  fi; \
	  (command -v shasum >/dev/null && shasum -a 256 "$(ELF)" | sed 's/^/[build] sha256=/') || true; \
	fi

# Flash and run (prints defmt logs via probe-rs)
run:
	$(CARGO) run --release $(CARGO_TARGET_FLAGS) --manifest-path $(CRATE_DIR)/Cargo.toml

# Force rebuild before flashing (useful when怀疑缓存)
run-force: clean run

# Attach to the running target with release ELF symbols loaded
attach: build
	@echo "[attach] chip=$(CHIP) probe=$(PROBE_ADDR) elf=$(ELF)"
	$(PROBE) attach --chip $(CHIP) --probe $(PROBE_ADDR) $(ELF)

# Reset the target MCU
reset:
	@echo "[reset] chip=$(CHIP) probe=$(PROBE_ADDR)"
	$(PROBE) reset --chip $(CHIP) --probe $(PROBE_ADDR)

# Reset then attach in one go
reset-attach: reset attach

# Clean build artifacts for this crate
clean:
	@echo "[clean] crate=$(CRATE_DIR)"
	$(CARGO) clean --manifest-path $(CRATE_DIR)/Cargo.toml

# Print current configuration and ELF path/hashes without flashing
info:
	@echo "[info] chip=$(CHIP) probe=$(PROBE_ADDR)"
	@echo "[info] crate=$(CRATE_DIR) target=$(TARGET_TRIPLE)"
	@echo "[info] elf=$(ELF)"; \
	if [ -f "$(ELF)" ]; then \
	  (command -v shasum >/dev/null && shasum -a 256 "$(ELF)") || true; \
	else \
	  echo "[info] ELF not built"; \
	fi
