# UPS Main Controller (ESP32-S3)

This directory contains the UPS main controller firmware targeting `ESP32‑S3` (Rust + esp-hal for MVP; versions aligned with the reference project).

- Toolchain: Rust + esp-hal (Xtensa). Requires `espflash`, Xtensa toolchain, and `xtensa-esp32s3-none-elf` target.
- Hardware docs:
  - `docs/mcu_hardware.md` (schematic transcription + verified GPIO mapping)
  - `docs/fan_control_spec.md` (2-wire fan control specification)
  - `docs/fan_control_requirements.md` (current fan-control requirements & acceptance checklist)
  - `docs/archive/pwm_fan_control_circuit_design.md` (legacy 3-wire fan design for reference)
  - `docs/datasheets/HUSB305-01.md` + `.pdf` (Type‑C source controller STAT/PG behavior)
- Build:
  - `make -C firmware/ups-main build` (or `ups-build` at repo root)
  - Flash + monitor: `make -C firmware/ups-main run PORT=/dev/tty...` (or `ups-run` at repo root)
- Status: MVP bring‑up scaffolded (GPIO/I2C/SPI/LEDC init; beep + fan smoke test).

Pin mapping in code follows `docs/mcu_hardware.md` exactly:
- Buttons (pull‑up): GPIO0/1/2/4/5
- RESET#(TCA6408A): GPIO6 (output-high)
- INTn (open-drain, low‑active): GPIO7 (input pull‑up)
- I2C0: SDA=GPIO8 SCL=GPIO9 @400kHz
- SPI LCD: DC10 MOSI11 SCLK12 CS13 RST14
- USB2_PG: GPIO21 (input pull‑up)
- FAN: EN=GPIO39, PWM=GPIO40@25kHz
- BUZZER: GPIO38@2kHz
