# UPS Main Controller (ESP32-S3)

This directory is reserved for the UPS main controller firmware targeting `ESP32‑S3`.

- Toolchain: ESP-IDF (C/C++) for MVP; Rust (esp-idf-hal) optional later.
- Hardware docs:
  - `docs/mcu_hardware.md` (schematic transcription + verified GPIO mapping)
  - `docs/pwm_fan_control_circuit_design.md` (fan DC/PWM scheme, same as reference project)
  - `docs/datasheets/HUSB305-01.md` + `.pdf` (Type‑C source controller STAT/PG behavior)
- Status: To be initialized in subsequent tasks.

For now, this folder is a placeholder. Build and CI settings will be added later.
