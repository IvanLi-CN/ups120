# Repository Guidelines

## Project Structure & Module Organization

- `firmware/smart-battery/`: STM32L051C8T6 firmware (Rust + Embassy); entrypoint `src/main.rs`.
- `firmware/ups-main/`: ESP32S3 placeholder binary.
- `embassy/`, `bq76920/`, `sc8815/`: local dependencies (git submodules).
- `scripts/`: tooling (e.g., `probe_runner.sh`).  `docs/`, `models/`, `logs/` hold design notes and assets.
- Initialize dependencies after clone: `git submodule update --init --recursive`.

## Build, Test, and Development Commands

- Setup (once): `rustup target add thumbv6m-none-eabi` and install `probe-rs` and `llvm-tools-preview`.
- Root-level operations use Makefile targets only. Do NOT run Cargo from the repository root.
  - Build smart-battery (release): `make sb-build`
  - Flash & run smart-battery: `make sb-run`
  - Attach/Reset: `make sb-attach` / `make sb-reset`
  - Driver example (STM32G0C8U6): `make driver-demo-build` / `make driver-demo-run`
- Per-project usage: you may also `make -C firmware/smart-battery run` (or `build`, `attach`, `reset`).
- Format: run `cargo fmt` inside the specific crate directory. Optional lint: run `cargo clippy --target thumbv6m-none-eabi` inside that crate.
- Host-side tests for dependency crates: from that crate directory run `cargo test` (e.g., `bq76920`, `sc8815`).

## Prohibited Operations (Mandatory)

- Do NOT create or reintroduce a Cargo workspace at the repository root.
- Do NOT add or rely on a root-level `.cargo/config.toml` for targets or runners.
- Do NOT run `cargo build`, `cargo run`, or any firmware-related Cargo command from the repository root.
- All firmware builds, flashing, and attach/reset must be invoked via Makefiles (root Makefile delegates to the project Makefiles) or by running Cargo within the specific project directory.
- Keep projects fully independent. Cross-crate paths must be explicit within each crate’s `Cargo.toml`; never depend on root workspace injection.

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

- Probe address and chip type are configured in the project’s own files (e.g., `firmware/smart-battery/.cargo/config.toml` or its Makefile). Override locally via environment variables (e.g., `PROBE_ADDR=XXXX make run`) instead of committing personal IDs.
