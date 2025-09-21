SHELL := /bin/sh

# Default chip for driver demo; override with `make CHIP=STM32G031C8 driver-demo-run`
CHIP ?= STM32G031C8

.PHONY: help sb-build sb-run sb-attach sb-reset driver-demo-build driver-demo-run clean

help:
	@echo "Root Makefile – use per-project make targets"
	@echo "Targets:"
	@echo "  sb-build         Build smart-battery firmware (release)"
	@echo "  sb-run           Flash+run smart-battery via probe-rs"
	@echo "  sb-attach        Attach to running target"
	@echo "  sb-reset         Reset target MCU"
	@echo "  driver-demo-build  Build STM32G0C8U6 demo (driver example)"
	@echo "  driver-demo-run    Flash+run STM32G0C8U6 demo (requires probe-rs)"

# Delegate to firmware/smart-battery/Makefile
sb-build:
	$(MAKE) -C firmware/smart-battery build

sb-run:
	$(MAKE) -C firmware/smart-battery run

sb-attach:
	$(MAKE) -C firmware/smart-battery attach

sb-reset:
	$(MAKE) -C firmware/smart-battery reset

# Driver demo: use cargo directly inside its folder (no cross-crate workspace)
driver-demo-build:
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo build --release

driver-demo-run: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && probe-rs run --chip $(CHIP) target/thumbv6m-none-eabi/release/smart-battery-stm32g0c8u6-demo

clean:
	@cd firmware/smart-battery && $(MAKE) clean || true
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo clean || true
