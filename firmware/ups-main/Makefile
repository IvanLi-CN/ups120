## ESP32-S3 (Xtensa) workflow for ups-main (Rust + esp-hal)
TARGET  ?= xtensa-esp32s3-none-elf
BIN     ?= ups-main
PROFILE ?= release
PORT    ?=
BAUD    ?= 115200
CHIP    ?= esp32s3
LOGFMT  ?= defmt

ifeq ($(PROFILE),release)
  CARGO_FLAGS := --release
else
  CARGO_FLAGS :=
endif

BINDIR := target/$(TARGET)/$(PROFILE)
ELF    := $(BINDIR)/$(BIN)

PORT_FLAG   := $(if $(PORT),--port $(PORT),)
BAUD_FLAG   := -B $(BAUD)
CHIP_FLAG   := --chip $(CHIP)
LOGFMT_FLAG := --log-format $(LOGFMT)
ESPFLASH_ARGS ?=

.PHONY: help env ports build clean run attach monitor

help:
	@echo "ups-main targets: build/run/attach/ports/env"

env:
	@echo TARGET=$(TARGET)
	@echo BIN=$(BIN)
	@echo PROFILE=$(PROFILE)
	@echo ELF=$(ELF)
	@echo PORT=$(PORT)
	@echo BAUD=$(BAUD)
	@echo CHIP=$(CHIP)
	@echo LOGFMT=$(LOGFMT)

ports:
	espflash scan-ports || espflash list-ports || true

build:
	cargo build $(CARGO_FLAGS)

clean:
	cargo clean

run: build
	espflash flash $(ELF) --monitor $(PORT_FLAG) $(BAUD_FLAG) $(CHIP_FLAG) $(LOGFMT_FLAG) $(ESPFLASH_ARGS)

attach: $(ELF)
	espflash monitor $(PORT_FLAG) $(BAUD_FLAG) $(CHIP_FLAG) $(LOGFMT_FLAG) --elf $(ELF) $(ESPFLASH_ARGS)

monitor: attach

