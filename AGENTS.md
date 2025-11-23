# Repository Guidelines

## Project Structure & Module Organization

- `firmware/smart-battery/`: STM32L051C8T6 firmware (Rust + Embassy); entrypoint `src/main.rs`.
- `firmware/ups-main/`: ESP32S3 UPS Main firmware (active development), entrypoint `src/main.rs`.
- `embassy/`, `bq76920/`, `sc8815/`: local dependencies (git submodules).
- `scripts/`: tooling (e.g., `probe_runner.sh`).  `docs/`, `models/`, `logs/` hold design notes and assets.
- Initialize dependencies after clone: `git submodule update --init --recursive`.

## Build, Test, and Development Commands

- Setup (once): `rustup target add thumbv6m-none-eabi` and install `probe-rs` and `llvm-tools-preview`.
- Root-level operations use Makefile targets only. Do NOT run Cargo from the repository root.
  - Build smart-battery (release): `make sb-build`
  - Build ups-main (release): `make -C firmware/ups-main build`
  - Driver example (STM32G0C8U6): `make driver-demo-build`
- Per-project Make targets are for builds only; on-target run/flash/reset/monitor must go through `mcu-agentd` (no `run/attach/reset` Make targets).
- Format: run `cargo fmt` inside the specific crate directory. Optional lint: run `cargo clippy --target thumbv6m-none-eabi` inside that crate.
- Host-side tests for dependency crates: from that crate directory run `cargo test` (e.g., `bq76920`, `sc8815`).

## Hardware Flashing & Logging (mcu-agentd)

- All on-target flash/reset/run monitoring/log queries must go through `tools/mcu-agentd` (see `tools/mcu-agentd/README.md`). Do not call `espflash`, `probe-rs`, or Make `run/attach/reset` targets directly.
- Daemon control: `just agentd start|status|stop`; ensure `status` is healthy before use.
- Port selection: `just agentd set-port esp32|stm32` (interactive or explicit port/Probe ID); verify with `get-port`.
- Flash/reset: `just agentd flash --mcu esp32|stm32 --elf <path>` or `just agentd reset --mcu ...`; commands pause/resume background monitoring automatically.
- Live logs: `just agentd monitor esp32|stm32` (supports `--duration`/`--lines`); history via `just agentd logs ...`.
- Default ELF paths: ESP32 `firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main`; STM32 `target/thumbv6m-none-eabi/release/smart-battery`. Build them first if missing.

## Prohibited Operations (Mandatory)

- Do NOT create or reintroduce a Cargo workspace at the repository root.
- Do NOT add or rely on a root-level `.cargo/config.toml` for targets or runners.
- Do NOT run `cargo build`, `cargo run`, or any firmware-related Cargo command from the repository root.
- All on-target flash/reset/monitor/log capture must use `just agentd ...`; do not use `make sb-run/sb-attach/sb-reset`, `make -C ... run/attach/reset`, or raw `espflash`/`probe-rs`.
- All firmware builds must be invoked via Makefiles (root Makefile delegates to the project Makefiles) or by running Cargo within the specific project directory.
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

- Probe address and chip type are configured in each project (e.g., `firmware/smart-battery/.cargo/config.toml` or its Makefile). You may override locally via env vars (e.g., `PROBE_ADDR=XXXX`) for builds, but on-target flashing/monitoring still must use `just agentd ...`.

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
