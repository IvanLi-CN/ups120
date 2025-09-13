# UPS120 Firmware (Split Architecture)

This repository now hosts two firmware targets under a unified workspace:

- Smart Battery Controller: `STM32L051C8T6` (battery BMS + charger orchestration)
- UPS Main Controller: `ESP32S3FH4R2` (system coordination; placeholder for now)

Refer to [WORKFLOW.md](WORKFLOW.md) and [DESIGN_MEMORANDUM.md](DESIGN_MEMORANDUM.md) for project context.

## Repository Layout

- `firmware/smart-battery/`: Rust + Embassy firmware for `STM32L051C8T6`
- `firmware/ups-main/`: Placeholder for `ESP32S3FH4R2` UPS main controller
- `embassy/`, `bq76920/`, `sc8815/`: Local dependencies and submodules

## Smart Battery (STM32L051C8T6)

- MCU: `STM32L051C8T6` (Cortex‑M0+)
- BMS: `BQ76920`
- Charger: `SC8815`
- Embassy setup: async I2C shared-bus; no USB on L0

I2C default pins (adjust to your board):
- `I2C1_SCL`: `PB6`
- `I2C1_SDA`: `PB7`

GPIO default mapping (adjust as needed):
- `PA0`: `SC8815_PSTOP` (active low to enable charging)
- `PA5`: Status LED (open-drain, active low)
- `PB9`: Discharge enable input (low = enable)
- `PA1`: Charge allow input (low = allow)

### Build & Run

From `firmware/smart-battery`:

1) Ensure Rust targets installed: `rustup target add thumbv6m-none-eabi`
2) Flash/Run with probe-rs: `cargo run` (uses local `.cargo/config.toml`)

Config uses `probe-rs` runner with chip `STM32L051C8Tx`. Update if your package differs.

### Notes on Port

- Migrated from `STM32G431` to `STM32L051C8T6` feature flags in `embassy-stm32`.
- Removed USB stack/tasks; L051 parts do not provide USB FS.
- I2C runs at 100 kHz async; DMA channel placeholders may be adjusted per board.

## UPS Main (ESP32S3FH4R2)

`firmware/ups-main` is a placeholder. ESP‑IDF (C/C++) or Rust (esp‑idf‑hal) integration will be added later.

## License

MIT — see [LICENSE](LICENSE).
