# Repository Guidelines

## Project Structure & Module Organization

- `firmware/smart-battery/`: STM32L051C8T6 firmware (Rust + Embassy); entrypoint `src/main.rs`.
- `firmware/ups-main/`: ESP32S3 placeholder binary.
- `embassy/`, `bq76920/`, `sc8815/`: local dependencies (git submodules).
- `scripts/`: tooling (e.g., `probe_runner.sh`).  `docs/`, `models/`, `logs/` hold design notes and assets.
- Initialize dependencies after clone: `git submodule update --init --recursive`.

## Build, Test, and Development Commands

- Setup (once): `rustup target add thumbv6m-none-eabi` and install `probe-rs` and `llvm-tools-preview`.
- Workspace build (default: smart-battery): `cargo build --release`.
- Flash & run (smart-battery): `cargo run -p smart-battery` (uses `.cargo/config.toml` runner), or from `firmware/smart-battery`: `make run` / `make attach` / `make reset`.
- Format: `cargo fmt`. Optional lint: `cargo clippy -p smart-battery --target thumbv6m-none-eabi`.
- Host-side tests (dependency crates): `cargo test -p bq769x0-async-rs`, `cargo test -p sc8815`.

## Coding Style & Naming Conventions

- Rust edition 2024; 4‑space indentation; keep modules `snake_case`, types `CamelCase`, constants `SCREAMING_SNAKE_CASE`.
- Run `cargo fmt` before commit; import grouping governed by `rustfmt.toml`.
- Use `defmt` for firmware logging (e.g., `defmt::info!`); keep log noise at or below `info` by default.

## Testing Guidelines

- Firmware targets are `no_std` and run on hardware; prefer hardware‑in‑the‑loop via `cargo run` (RTT/defmt output).
- Place pure‑logic tests in dependency crates or new host‑runnable modules; name files `*_test.rs` or inline `mod tests` with `#[cfg(test)]`.
- Aim for unit coverage of parsing/math and critical safety checks in `bq76920`/`sc8815` crates.

## Commit & Pull Request Guidelines

- Use Conventional Commits in English (enforced by `commitlint.config.cjs`):
  - Example: `feat(smart-battery): publish INA226 power readings`
  - Header ≤ 72 chars; include scope when useful.
- Pre-commit hooks: `lefthook` runs `cargo fmt` on staged Rust files.
- PRs: describe intent and impact, link issues, include testing notes (hardware used, probe ID), and attach logs/screenshots when relevant.

## Security & Configuration Tips

- Probe address and chip type are set in `.cargo/config.toml` and `firmware/smart-battery/.cargo/config.toml`. Override locally (e.g., `PROBE_ADDR=XXXX make run`) instead of committing personal IDs.
