# UPS120 Firmware (Split Architecture)

This repository now hosts two firmware targets under a unified workspace:

- Smart Battery Controller: `STM32L051C8T6` (battery BMS + charger orchestration)
- UPS Main Controller: `ESP32S3FH4R2` (system coordination; active development)

Refer to [WORKFLOW.md](WORKFLOW.md) and [DESIGN_MEMORANDUM.md](DESIGN_MEMORANDUM.md) for project context.

Flashing/reset/log capture is routed through `mcu-agentd` (see `docs/mcu-agentd.md`).

## Repository Layout

- `firmware/smart-battery/`: Rust + Embassy firmware for `STM32L051C8T6`
- `firmware/ups-main/`: Rust firmware for `ESP32S3FH4R2` UPS main controller (active development)
- `embassy/`, `bq76920/`, `sc8815/`: Local dependencies and submodules

## Smart Battery (STM32L051C8T6)

- MCU: `STM32L051C8T6` (Cortex‑M0+)
- BMS: `BQ76920`
- Charger: `SC8815`
- Embassy setup: async I2C shared-bus; no USB on L0

I2C pins (per CubeMX .ioc):
- `I2C2` (INNER bus): `PB10` = SCL, `PB11` = SDA
- `I2C1` (SMBus alert mode enabled): `PB6` = SCL, `PB7` = SDA, `PB5` = SMBA

GPIO mapping (netlist‑accurate; the `.ioc` file is slightly stale):
- `PA9`: `PSTOP` (GPIO Output) — HIGH = power stage gated, LOW = enable
- `PA10`: `CE` (GPIO Output) — LOW = charger enabled
- `PA5`: `LEDK` (TIM2_CH1 alternate, open‑drain LED in firmware)
- `PA2`: `PCHG_EN` (GPIO Output)  *(per .ioc; see PCB netlist for the exact pad)*
- `PA0..PA3`: `ADC_IN0..3` for the 4× NTC network (`TS45`/`TS34`/`TS23`/`TS12` → pack temperatures)
- `PH0`: `EXIT_SHIPMODE` (GPIO Output) — routed via D3 clamp onto BQ76920 `TS1`, used to wake from SHIP mode
- `PB1`: `ALERT` (EXTI1)
- `PB2`: `INNER_INT` (EXTI2)

### Build & Run

From the repository root:

1) Ensure targets are installed: `rustup target add thumbv6m-none-eabi`
2) Build: `just sb-build`
3) Flash: `just sb-flash`
4) Monitor: `just sb-monitor`

Notes:
- The on-target workflow is mediated by `mcu-agentd` (do not call `probe-rs` directly).
- If you use a custom `CARGO_TARGET_DIR`, update `mcu-agentd.toml` accordingly.

### Notes on Port

- Migrated from `STM32G431` to `STM32L051C8T6` feature flags in `embassy-stm32`.
- Removed USB stack/tasks; L051 parts do not provide USB FS.
- I2C runs at 100 kHz async; DMA channel placeholders may be adjusted per board.

## UPS Main (ESP32S3FH4R2)

`firmware/ups-main` is an actively developed Rust firmware using `esp-hal` on ESP32-S3. System coordination, fan control, and I2C integrations (e.g., smart‑battery telemetry) are incrementally landing here.

### Build & Run

From the repository root:

1) Build: `just ups-build`
2) Flash: `just ups-flash`
3) Monitor: `just ups-monitor`

## License

MIT — see [LICENSE](LICENSE).
