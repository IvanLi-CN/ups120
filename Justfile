set shell := ["/bin/sh", "-c"]

# Generic agentd passthrough (release)
agentd +args:
	cd tools/mcu-agentd && cargo run --release -- {{args}}

agentd-start:
	just agentd start

agentd-status:
	just agentd status

agentd-stop:
	just agentd stop

agentd-set-port mcu path="":
	if [ -z "{{path}}" ]; then \
	  cd tools/mcu-agentd && cargo run --release -- set-port {{mcu}}; \
	else \
	  cd tools/mcu-agentd && cargo run --release -- set-port {{mcu}} {{path}}; \
	fi

agentd-get-port mcu:
	cd tools/mcu-agentd && cargo run --release -- get-port {{mcu}}

# Smart-battery (STM32)
sb-build:
	set -eu; \
	cd firmware/smart-battery; \
	TARGET_DIR=${CARGO_TARGET_DIR:-$(pwd)/../../target}; \
	PROBE_ADDR=${PROBE_ADDR:-$([ -x ../../scripts/ensure_stm32_probe.sh ] && ../../scripts/ensure_stm32_probe.sh || echo "")}; \
	DEFMT_LOG=${DEFMT_LOG:-info}; \
	CARGO_TARGET_DIR="$TARGET_DIR" PROBE_ADDR="$PROBE_ADDR" DEFMT_LOG="$DEFMT_LOG" cargo build --release --target thumbv6m-none-eabi

sb-flash: sb-build
	just agentd flash stm32 --elf firmware/smart-battery/target/thumbv6m-none-eabi/release/smart-battery

sb-reset:
	just agentd reset stm32

sb-monitor:
	just agentd monitor stm32 --duration 30s --lines 200

# UPS main (ESP32-S3)
ups-build:
	cd firmware/ups-main && cargo build --release

ups-flash: ups-build
	just agentd flash esp32 --elf firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main

ups-reset:
	just agentd reset esp32

ups-monitor:
	just agentd monitor esp32 --duration 30s --lines 200

# Driver demo (STM32G0)
driver-demo-build:
	set -eu; \
	cd libs/smart-battery-driver/examples/stm32g0; \
	DEMO_FLAGS=""; [ -n "${DEMO_FEATURES:-}" ] && DEMO_FLAGS="--features ${DEMO_FEATURES}"; \
	cargo build --release $DEMO_FLAGS

# Docs
docs-build:
	set -eu; \
	cd embassy/docs; \
	asciidoctor -d book -D book/ index.adoc; \
	cp -r images book

docs-clean:
	rm -rf embassy/docs/book
