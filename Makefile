SHELL := /bin/sh

# Optional parameters (can be left unset)
CHIP ?= STM32G031C8
PROBE ?=
DEMO_FEATURES ?=
DRIVER_PROBE_FLAGS := $(if $(PROBE),--non-interactive --probe $(PROBE),)

.PHONY: help \
        sb-build sb-run sb-attach sb-reset sb-reset-attach \
        driver-demo-build driver-demo-run driver-demo-attach driver-demo-reset-attach \
        ups-build ups-run ups-attach ups-ports ups-clean ups-env \
        clean

help:
	@echo "Root Makefile"
	@echo "  sb-build            Build smart-battery firmware"
	@echo "  sb-run              Flash+run smart-battery"
	@echo "  sb-attach           Attach to smart-battery"
	@echo "  sb-reset            Reset smart-battery"
	@echo "  sb-reset-attach     Reset then attach smart-battery"
	@echo "  driver-demo-build   Build STM32G0 demo (features via DEMO_FEATURES)"
	@echo "  driver-demo-run     Flash+run STM32G0 demo (no auto-build)"
	@echo "  driver-demo-attach  Attach to STM32G0 demo (optional CHIP/PROBE)"
	@echo "  driver-demo-reset-attach  Reset then attach STM32G0 demo"
	@echo "  ups-build           Build UPS main (ESP32-S3)"
	@echo "  ups-run             Flash+monitor UPS main (optional PORT/BAUD/LOGFMT/ESPFLASH_ARGS)"
	@echo "  ups-attach          Monitor UPS main (requires prior build)"
	@echo "  ups-ports           List serial ports via espflash"
	@echo "  ups-clean           Clean UPS main build artifacts"
	@echo "  ups-env             Show UPS main env (target/paths)"
	@echo "Vars: CHIP=$(CHIP) PROBE=$(PROBE) DEMO_FEATURES=$(DEMO_FEATURES)"
	@echo "UPS Vars (forwarded if set): PORT=$(PORT) BAUD=$(BAUD) LOGFMT=$(LOGFMT) ESPFLASH_ARGS=$(ESPFLASH_ARGS)"

# Delegate to firmware/smart-battery/Makefile (probe config handled there; no arg passthrough)
sb-build:
	$(MAKE) -C firmware/smart-battery build

sb-run:
	$(MAKE) -C firmware/smart-battery run

sb-attach:
	$(MAKE) -C firmware/smart-battery attach

sb-reset:
	$(MAKE) -C firmware/smart-battery reset

# Reset then attach smart-battery (delegates to project Makefile)
sb-reset-attach:
	$(MAKE) -C firmware/smart-battery reset-attach

# Driver demo (optional arguments; not required)
driver-demo-build:
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo build --release $(if $(DEMO_FEATURES),--features $(DEMO_FEATURES),)

driver-demo-run:
	@cd libs/smart-battery-driver/examples/stm32g0 && \
	  probe-rs run $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

driver-demo-attach: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && probe-rs attach $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

driver-demo-reset-attach: driver-demo-build
	@cd libs/smart-battery-driver/examples/stm32g0 && probe-rs reset $(DRIVER_PROBE_FLAGS) --chip $(CHIP) && \
		probe-rs attach $(DRIVER_PROBE_FLAGS) --chip $(CHIP) --log-format oneline target/thumbv6m-none-eabi/release/smart-battery-driver-stm32g0-demo

## UPS main (ESP32-S3) aliases — delegate to firmware/ups-main/Makefile
ups-build:
	$(MAKE) -C firmware/ups-main build

ups-run:
	$(MAKE) -C firmware/ups-main run \
	  $(if $(PORT),PORT=$(PORT),) \
	  $(if $(BAUD),BAUD=$(BAUD),) \
	  $(if $(LOGFMT),LOGFMT=$(LOGFMT),) \
	  $(if $(ESPFLASH_ARGS),ESPFLASH_ARGS="$(ESPFLASH_ARGS)",)

ups-attach:
	$(MAKE) -C firmware/ups-main attach \
	  $(if $(PORT),PORT=$(PORT),) \
	  $(if $(BAUD),BAUD=$(BAUD),) \
	  $(if $(LOGFMT),LOGFMT=$(LOGFMT),) \
	  $(if $(ESPFLASH_ARGS),ESPFLASH_ARGS="$(ESPFLASH_ARGS)",)

ups-ports:
	$(MAKE) -C firmware/ups-main ports

ups-clean:
	$(MAKE) -C firmware/ups-main clean

ups-env:
	$(MAKE) -C firmware/ups-main env

clean:
	@cd firmware/smart-battery && $(MAKE) clean || true
	@cd libs/smart-battery-driver/examples/stm32g0 && cargo clean || true
