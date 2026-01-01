# Repository Guidelines

## Project Structure & Module Organization

- `firmware/smart-battery/`: STM32L051C8T6 firmware (Rust + Embassy); entrypoint `src/main.rs`.
- `firmware/ups-main/`: ESP32S3 UPS Main firmware (active development), entrypoint `src/main.rs`.
- `embassy/`, `bq76920/`, `sc8815/`: local dependencies (git submodules).
- `scripts/`: tooling (e.g., `probe_runner.sh`).  `docs/`, `models/`, `logs/` hold design notes and assets.
- Initialize dependencies after clone: `git submodule update --init --recursive`.

## Build, Test, and Development Commands

- Setup (once): `rustup target add thumbv6m-none-eabi` and install `probe-rs` and `llvm-tools-preview`.
- Use `just` exclusively (no Makefiles remain; do not invoke `make`).
  - Build smart-battery (release): `just sb-build`
  - Flash/reset/monitor smart-battery: `just sb-flash` / `just sb-reset` / `just sb-monitor`
  - Build ups-main (release): `just ups-build`
  - Flash/reset/monitor ups-main: `just ups-flash` / `just ups-reset` / `just ups-monitor`
  - Driver example (STM32G0C8U6) build: `just driver-demo-build`
  - Docs (embassy/docs) build/clean: `just docs-build` / `just docs-clean`
- On-target run/flash/reset/monitor must go through `mcu-agentd` via the `just` wrappers; do not call espflash/probe-rs directly.
- Format: run `cargo fmt` inside the specific crate directory. Optional lint: run `cargo clippy --target thumbv6m-none-eabi` inside that crate.
- Host-side tests for dependency crates: from that crate directory run `cargo test` (e.g., `bq76920`, `sc8815`).

## Hardware Flashing & Logging (mcu-agentd)

- **Single daemon, `just` wrappers**: All flash/reset/monitor/log actions must use `mcu-agentd` via the `just` wrappers. Do not call `espflash`/`probe-rs` directly.
- **Project config**: repo root must provide `mcu-agentd.toml` (see `docs/mcu-agentd.md`).
- **Daemon control**: `just agentd-start | agentd-status | agentd-stop` (equivalent to `mcu-agentd {start|status|stop}`).
- **Port/probe cache (user approval required)**:
  - Cache files: `.esp32-port`, `.stm32-port`.
  - Set (preferred): `mcu-agentd selector set esp32 <port>` / `mcu-agentd selector set stm32 <probe-id>`.
  - Get: `mcu-agentd selector get esp32|stm32`.
- **Flash / reset / monitor**:
  - Flash: `mcu-agentd flash esp32` / `mcu-agentd flash stm32` (ELF comes from `mcu-agentd.toml`; build first if missing).
  - Reset: `mcu-agentd reset esp32|stm32` (reset only).
  - Monitor: `mcu-agentd monitor esp32|stm32` (`--from-start` / `--reset` optional; streams until Ctrl+C).
  - Logs: `mcu-agentd logs all --tail 200 --sessions` aggregates meta + recent sessions.
- **Default ELF paths**: ESP32 `firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main`; STM32 `target/thumbv6m-none-eabi/release/smart-battery`. Build first if absent.
- **Version check**: ensure boot logs print firmware version/hash and confirm it matches the locally built image before validating hardware results.
- **Web UI (optional)**: register the project once via `mcu-managerd projects add .`, then open `open "$(mcu-agentd web)"`.
- **Bring-up order**: power/flash/run ESP32 (`ups-main`) first, then flash/reset STM32 (smart-battery). STM32 I2C1 is an external slave; if the host is absent or the bus is floating, the STM32 will see repeated NACKs from that bus.
- **STM32 gating**: keep STM32 held in reset or unpowered until ESP32 has completed its post-boot init (I2C host up, rails configured). STM32 depends on ESP32's initialization to enter controlled mode; if STM32 boots first, it will remain in an uninitialized/NACK loop.
- **User interaction rule**: agentd uses cached or auto-detected ports first; only ask the user if unresolved. Never switch probes/ports without consent.

## Prohibited Operations (Mandatory)

- Do NOT create or reintroduce a Cargo workspace at the repository root.
- Do NOT add or rely on a root-level `.cargo/config.toml` for targets or runners.
- Do NOT run `cargo build`, `cargo run`, or any firmware-related Cargo command from the repository root; use `just` recipes that cd into the crate.
- Do NOT use `make`; Makefiles have been removed in favor of `just`.
- All on-target flash/reset/monitor/log capture must use `just agentd ...`; do not use raw `espflash`/`probe-rs`.
- Keep projects fully independent. Cross-crate paths must be explicit within each crate’s `Cargo.toml`; never depend on root workspace injection.

## Coding Style & Naming Conventions

- Rust edition 2024; 4‑space indentation; keep modules `snake_case`, types `CamelCase`, constants `SCREAMING_SNAKE_CASE`.
- Run `cargo fmt` before commit; import grouping governed by `rustfmt.toml`.
- Use `defmt` for firmware logging (e.g., `defmt::info!`); keep log noise at or below `info` by default.

## Testing Guidelines

- Firmware targets are `no_std` and run on hardware; for HIL, flash with `just agentd flash ...` then observe with `just agentd monitor/logs`. Never `cargo run` on targets.
- Place pure‑logic tests in dependency crates or new host‑runnable modules; name files `*_test.rs` or inline `mod tests` with `#[cfg(test)]`.
- Aim for unit coverage of parsing/math and critical safety checks in `bq76920`/`sc8815` crates.

## Commit & Pull Request Guidelines

- Use Conventional Commits in English (enforced by `commitlint.config.cjs`):
  - Example: `feat(smart-battery): publish INA226 power readings`
  - Header ≤ 72 chars; include scope when useful.
- Pre-commit hooks: `lefthook` runs `cargo fmt` on staged Rust files.
- PRs: describe intent and impact, link issues, include testing notes (hardware used, probe ID), and attach logs/screenshots when relevant.

## Security & Configuration Tips

- Probe address and chip type are configured in each project (e.g., `firmware/smart-battery/.cargo/config.toml`); Makefiles are gone. You may override locally via env vars (e.g., `PROBE_ADDR=XXXX`) for builds, but on-target flashing/monitoring still must use `just agentd ...`.

## I2C Device Instance Management (Supplement)

- Single instance per chip: Only one driver instance per physical I2C chip at a time. Share the bus using `embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice` instead of cloning chip driver instances.
- Option-wrapped ownership: Manage chip drivers with `Option<T>` so you can explicitly drop the old instance before creating a new one. Rebuild flow: set the holder to `None` (or let it go out of scope), call `release()` if provided to return the bus, then construct the new instance and store it back as `Some(new)`.
- Initialize once per power cycle: Call `init()` exactly once after power-up. Later sessions should only gate power stages, tweak configuration, and start/stop ADCs. If you intentionally power-cycle the device (e.g., CE low), that starts a new cycle and allows a fresh `init()`.
- Power gating policy: Prefer keeping the chip online across sessions (e.g., keep `CE=High`, stop power stage via `PSTOP`) to avoid re-initialization and preserve configuration unless a quiesce/failsafe policy requires a full power-down.
- Review checklist for PRs touching I2C/driver code:
  - Only one instance exists per chip at any time.
  - Instance holder uses `Option<T>` to allow explicit teardown before rebuild.
  - `release()` is called before re-creating an instance when applicable.
  - `init()` is not called repeatedly in normal operation.
  - Bus sharing uses `I2cDevice` rather than multiple chip instances.

Example pattern:

```rust
// Holder
static mut SC: Option<SC8815<I2cDev>> = None;

// Rebuild safely
if let Some(old) = unsafe { SC.take() } {
    let _i2c = old.release(); // free bus handle
}
let mut sc = SC8815::new(i2c, addr);
if !SC_INIT_DONE.load(Relaxed) { sc.init().await?; SC_INIT_DONE.store(true, Relaxed); }
unsafe { SC = Some(sc); }
```
