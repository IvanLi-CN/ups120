SHELL := /bin/sh

# 可选参数（保留，但用户可直接不带参数使用）
CHIP ?= STM32G031C8
PROBE ?=
DEMO_FEATURES ?=
DRIVER_PROBE_FLAGS := $(if $(PROBE),--non-interactive --probe $(PROBE),)

.PHONY: help sb-build sb-run sb-attach sb-reset \
        driver-demo-build driver-demo-run driver-demo-attach driver-demo-reset-attach clean

help:
	@echo "Root Makefile"
	@echo "  sb-build            Build smart-battery firmware"
	@echo "  sb-run              Flash+run smart-battery"
	@echo "  sb-attach           Attach to smart-battery"
	@echo "  sb-reset            Reset smart-battery"
	@echo "  driver-demo-build   Build STM32G0 demo (features via DEMO_FEATURES)"
	@echo "  driver-demo-run     Flash+run STM32G0 demo (CHIP/PROBE 可选)"
	@echo "  driver-demo-attach  Attach to STM32G0 demo (CHIP/PROBE 可选)"
	@echo "  driver-demo-reset-attach  Reset then attach STM32G0 demo"
	@echo "Vars: CHIP=$(CHIP) PROBE=$(PROBE) DEMO_FEATURES=$(DEMO_FEATURES)"

# Delegate to firmware/smart-battery/Makefile（其内已硬编码目标探针；这里不透传参数）
sb-build:
	$(MAKE) -C firmware/smart-battery build

sb-run:
	$(MAKE) -C firmware/smart-battery run

sb-attach:
	$(MAKE) -C firmware/smart-battery attach

sb-reset:
	$(MAKE) -C firmware/smart-battery reset

# Driver demo（允许可选参数，但不强制）
driver-demo-build:
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo build --release $(if $(DEMO_FEATURES),--features $(DEMO_FEATURES),)

driver-demo-run: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && \
	  echo "[demo] ELF=target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo" && \
	  ( (command -v gstat >/dev/null && gstat -c '[demo] mtime=%y size=%s' target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo) || \
	    (stat -f '[demo] mtime=%Sm size=%z' -t '%Y-%m-%d %H:%M:%S' target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo) ); \
	  (command -v shasum >/dev/null && shasum -a 256 target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo | sed 's/^/[demo] sha256=/') || true; \
	  probe-rs run $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

driver-demo-attach: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && probe-rs attach $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

driver-demo-reset-attach: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && probe-rs reset $(DRIVER_PROBE_FLAGS) --chip $(CHIP) && \
		probe-rs attach $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

clean:
	@cd firmware/smart-battery && $(MAKE) clean || true
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo clean || true
