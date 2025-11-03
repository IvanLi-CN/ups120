# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

UPS120 是一个双固件架构的 UPS 项目：
- **Smart Battery Controller** (STM32L051C8T6): 基于 Rust + Embassy 的电池 BMS + 充电器编排控制器
- **UPS Main Controller** (ESP32-S3): 系统协调控制器（目前为占位实现）

两个固件是完全独立的项目，不使用 Cargo workspace。

## Build & Flash Commands

### Prerequisites
```bash
# Install Rust targets
rustup target add thumbv6m-none-eabi
rustup component add llvm-tools-preview rust-src

# Install tools
cargo install probe-rs espflash
```

### Smart Battery (STM32L051C8T6)
```bash
# Build firmware (release profile optimized for size)
make sb-build

# Flash and run (requires probe-rs and J-Link/ST-Link)
make sb-run

# Attach to running firmware (view defmt logs via RTT)
make sb-attach

# Reset and attach
make sb-reset-attach
```

项目级命令（从 firmware/smart-battery/ 目录）:
```bash
cd firmware/smart-battery
make build    # or make run, make attach, make reset
```

### UPS Main (ESP32-S3)
```bash
# Build
make ups-build

# Flash and monitor
make ups-run

# Monitor only (requires prior build)
make ups-attach

# List available serial ports
make ups-ports
```

可选参数（通过环境变量传递）:
```bash
make ups-run PORT=/dev/ttyUSB0 BAUD=921600 LOGFMT=defmt
```

### 重要约束
- **禁止在仓库根目录运行 `cargo build` 或 `cargo run`**
- **禁止创建根级 Cargo workspace**
- 所有构建必须通过 Makefile 或在具体项目目录内执行

## Architecture

### Smart Battery Firmware Structure

主控制流程 (`src/main.rs`):
- **I2C2 (内部总线)**: 连接 BQ76920 (BMS) 和 SC8815 (充电器)
  - BQ76920: 固定地址 0x08 (BQ7692003PWR variant with CRC)
  - 使用 Embassy I2C shared-bus 模式实现异步共享访问
- **I2C1 (SMBus)**: Smart Battery 对外接口，作为 I2C slave
  - 支持 SMBus alert 模式 (SMBA on PB5)

关键任务（Embassy executor）:
- `bq76920_task`: BMS 监控，配置验证，MOS 管控制，故障检测
- `sc8815_task`: 充电器控制，根据 BMS 状态决定充放电
- `i2c1_slave`: SMBus 从设备接口，响应主机查询
- `leds4_task`: 4 颗状态指示 LED（电量、充电、故障、活动）
- `failsafe`: 看门狗和失效保护逻辑
- `sleep_manager`: 低功耗模式管理

GPIO 映射（参考 smart-battery.ioc）:
- PA9: PSTOP (power stage gate control)
- PA10: CE (charger enable)
- PA5: LEDK (TIM2_CH1 PWM LED)
- PB1: ALERT (EXTI1 from BQ76920)
- PB2: INNER_INT (EXTI2 interrupt mux)

关键设计决策:
- **BQ76920 配置验证**: 初始化时必须回读并验证关键安全寄存器（OV_TRIP, UV_TRIP, PROTECT1/2/3, CC_CFG）
- **MOS 管默认使能**: 配置验证成功后，主动使能 CHG_ON 和 DSG_ON
- **共享状态通信**: 使用 `embassy_sync` 的 Channel/Signal/Mutex 在任务间同步状态
- **Flash 优化**: dev/release 均使用 LTO 'fat' 和 opt-level 'z'

### UPS Main Firmware Structure

当前实现（占位）:
- 基于 esp-hal 的裸机程序（非 Embassy executor）
- 包含 TSENS（ESP32-S3 内部温度传感器）驱动测试
- GPIO 配置: 按钮输入、TCA6408A I/O 扩展、SPI/I2C 外设初始化

目标架构（待实现）:
- 系统协调器，可能包含 WiFi/蓝牙通信、UI 控制、多电池管理

## Testing

### Hardware-in-the-Loop Testing
固件项目是 `no_std` 嵌入式程序，主要通过硬件测试:
```bash
# Smart Battery: 通过 RTT 查看 defmt 日志
make sb-run
# 或
make sb-attach  # 如果已经刷写

# UPS Main: 通过 UART 查看日志
make ups-run
```

### Unit Tests
纯逻辑测试放在依赖 crate 中:
```bash
cd bq76920 && cargo test
cd sc8815 && cargo test
```

## Code Style

- Rust edition 2024, 4-space indentation
- `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants
- 使用 `defmt` 进行固件日志记录（而非 `log` crate）
- 提交前运行 `cargo fmt`（lefthook 会自动检查）

## Git & Commits

- **Conventional Commits** (enforced by commitlint):
  ```
  feat(smart-battery): add SOC estimation
  fix(ups-main): correct TSENS calibration
  docs: update CLAUDE.md build instructions
  ```
- **Commit message 必须使用英文**
- **禁止使用 `--no-verify` 绕过 pre-commit hooks**
- Pre-commit: `lefthook` 会运行 `cargo fmt` 检查

## Dependencies

### Git Submodules
本项目使用 git submodule 管理本地依赖:
```bash
# Clone 后初始化
git submodule update --init --recursive

# 更新 submodule
git submodule update --remote
```

Submodules:
- `embassy/`: Embassy async runtime (官方仓库)
- `bq76920/`: BQ769x0 BMS 驱动 (自维护 fork)
- `sc8815/`: SC8815 充电器驱动 (自维护)

### Probe Configuration

Smart Battery 的 probe 地址配置在 `firmware/smart-battery/.cargo/config.toml`:
```toml
runner = "probe-rs run --chip STM32L051C8Tx --probe <ID>"
```

本地覆盖（避免提交个人 probe ID）:
```bash
# 通过 Makefile 的 PROBE 变量
PROBE=<your-probe-id> make sb-run
```

## Key Files & Entry Points

- `firmware/smart-battery/src/main.rs`: Smart Battery 主入口
  - `#[embassy_executor::main]` async main
  - 初始化外设、启动所有任务
- `firmware/smart-battery/src/bq76920_task.rs`: BMS 任务逻辑
  - 配置验证、故障检测、MOS 管控制
- `firmware/smart-battery/src/sc8815_task.rs`: 充电器任务
  - 充电策略、基于 BMS 状态的使能控制
- `firmware/ups-main/src/main.rs`: UPS Main 主入口
  - `#[esp_hal::main]` 裸机入口

## Documentation

项目文档:
- `README.md`: 项目概览和快速开始
- `WORKFLOW.md`: MVP 业务流程详细说明（含 Mermaid 流程图）
- `DESIGN_MEMORANDUM.md`: 设计决策记录（如 BQ25730 cell_count 配置原理）
- `AGENTS.md`: 仓库贡献者指南（构建、测试、提交规范）
- `LED_STATUS_README.md`: LED 状态指示说明

硬件参考:
- `models/`: 3D 模型和机械设计文件
- `docs/`: 数据手册和参考文档
- `.ioc` 文件: CubeMX 项目（用于 GPIO/时钟配置参考，不直接用于构建）

## Common Pitfalls

1. **不要在根目录运行 Cargo 命令**: 始终使用 Makefile 或进入具体项目目录
2. **Probe ID 冲突**: 多个 probe 连接时需显式指定 `--probe <ID>`
3. **Flash 空间紧张**: STM32L051C8 只有 64KB，避免引入大型依赖或冗余日志
4. **I2C 共享总线**: 使用 Embassy shared-bus 时注意 `&'static Mutex` 生命周期
5. **Defmt 版本一致性**: 所有 crate 必须使用相同 defmt 版本以保证格式兼容

## ESP32-S3 Specific Notes

- Target: `xtensa-esp32s3-none-elf` (需要 Xtensa Rust toolchain)
- Flashing: `espflash` (通过 USB-UART 桥接，如 CP2102)
- 日志格式: 支持 defmt 格式化（通过 `esp-println` 的 defmt-espflash feature）
- 环境变量:
  ```bash
  DEFMT_LOG=info                        # 控制日志级别
  EMBASSY_EXECUTOR_TASK_ARENA_SIZE=65536  # 任务栈大小
  ```
