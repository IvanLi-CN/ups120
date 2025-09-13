CHIP = STM32L051C8Tx
TARGET_DIR = target/thumbv6m-none-eabi

.PHONY: attach attach-release reset reset-release reset-attach reset-attach-release

attach:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/smart-battery

attach-release:
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/smart-battery

reset:
	probe-rs reset --chip $(CHIP)

reset-release:
	probe-rs reset --chip $(CHIP)

reset-attach: reset
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/debug/smart-battery

reset-attach-release: reset-release
	probe-rs attach --chip $(CHIP) $(TARGET_DIR)/release/smart-battery
