CHIP = STM32G431CBUx
TARGET_DIR = target/thumbv7em-none-eabihf

.PHONY: attach attach-release reset reset-release reset-attach reset-attach-release

attach:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/ups120

attach-release:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/ups120

reset:
	probe-rs reset --chip $(CHIP)

reset-release:
	probe-rs reset --chip $(CHIP)

reset-attach: reset
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/ups120

reset-attach-release: reset-release
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/ups120