CHIP = RP2040
TARGET_DIR = target/thumbv6m-none-eabi

.PHONY: build build-release run run-release attach attach-release reset reset-release reset-attach reset-attach-release size size-release clean flash flash-release

# Build targets
build:
	cargo build

build-release:
	cargo build --release

# Run targets (build and flash)
run:
	cargo run

run-release:
	cargo run --release

# Debug attach targets
attach:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/ups120

attach-release:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/ups120

# Reset targets
reset:
	probe-rs reset --chip $(CHIP)

reset-release:
	probe-rs reset --chip $(CHIP)

# Combined reset and attach
reset-attach: reset
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/ups120

reset-attach-release: reset-release
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/ups120

# Size analysis
size:
	cargo size --bin ups120

size-release:
	cargo size --bin ups120 --release

# Flash without running
flash:
	probe-rs run --chip $(CHIP) --protocol swd $(TARGET_DIR)/debug/ups120

flash-release:
	probe-rs run --chip $(CHIP) --protocol swd $(TARGET_DIR)/release/ups120

# Clean build artifacts
clean:
	cargo clean

# Development helpers
fmt:
	cargo fmt

clippy:
	cargo clippy

check:
	cargo check

test:
	cargo test

# Combined development check
dev-check: fmt clippy check
	@echo "Development checks completed"
