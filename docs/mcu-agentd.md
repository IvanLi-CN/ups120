# UPS120 使用 mcu-agentd

本仓库使用外部项目 `mcu-agentd`/`mcu-managerd` 作为统一的 MCU 操作入口（flash/reset/monitor/logs + selector 缓存 + Web UI）。

## 目的与范围

- 统一管理两路 MCU：
  - UPS main：ESP32-S3（`firmware/ups-main`）
  - smart-battery：STM32L051（`firmware/smart-battery`）
- `mcu-agentd` 负责实际执行 `espflash`/`probe-rs` 并落盘日志；`mcu-managerd` 提供跨项目入口与 Web UI。

## 前置依赖

- 底层工具：
  - `espflash`（ESP32）
  - `probe-rs`（STM32）
- 全局安装（产物在 `$HOME/.cargo/bin`，确保在 `PATH` 中）：

  ```bash
  cargo install --path /Users/ivan/Projects/Ivan/mcu-agentd --bins
  ```

校验：

```bash
mcu-agentd --version
mcu-managerd --version
```

## 配置（`mcu-agentd.toml`）

在仓库根目录创建 `mcu-agentd.toml`，约定：

- `[project].id = "ups120"`
- `mcu_id` 固定使用 `esp32` / `stm32`
- selector 缓存文件复用仓库根：
  - ESP32：`.esp32-port`
  - STM32：`.stm32-port`

示例（按需调整路径）：

```toml
[project]
id = "ups120"

[agentd]
auto_start_daemon = true
tail_default = 200
non_interactive = false

[mcu.esp32]
backend = "espflash"
chip = "esp32s3"
artifact_elf = "firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main"
selector_cache_file = ".esp32-port"

[mcu.esp32.espflash]
log_format = "defmt"
after_flash = "no-reset"
skip_update_check = true

[mcu.stm32]
backend = "probe-rs"
chip = "STM32L051C8Tx"
artifact_elf = "target/thumbv6m-none-eabi/release/smart-battery"
selector_cache_file = ".stm32-port"

[mcu.stm32.probe_rs]
protocol = "swd"
speed_khz = 4000
connect_under_reset = false
```

校验配置：

```bash
mcu-agentd config validate
```

## selector（端口/探针缓存）

本仓库约定 selector 缓存文件是 `.esp32-port` 与 `.stm32-port`（每文件一行，内容为 selector 值）。

常用命令：

```bash
mcu-agentd selector list esp32
mcu-agentd selector set esp32 /dev/cu.usbmodemXXXX
mcu-agentd selector get esp32

mcu-agentd selector list stm32
mcu-agentd selector set stm32 0483:3748:SERIAL
mcu-agentd selector get stm32
```

说明：

- 若你选择 `selector set <mcu_id>`（无 VALUE）走交互模式，可能触发严格校验；在构建产物尚未生成时，推荐直接传入 VALUE。

## 项目注册与 Web UI

首次使用需显式注册项目（支持多项目切换与资源仲裁）：

```bash
mcu-managerd projects add .
```

打开 Web UI：

```bash
open "$(mcu-agentd web)"
```

## 常用命令

daemon 生命周期：

```bash
mcu-agentd start
mcu-agentd status
mcu-agentd stop
```

flash/reset/erase：

```bash
mcu-agentd flash esp32
mcu-agentd reset esp32

mcu-agentd flash stm32
mcu-agentd reset stm32

mcu-agentd erase esp32 --yes
mcu-agentd erase stm32 --yes
```

logs/monitor：

```bash
mcu-agentd logs all --tail 200 --sessions
mcu-agentd monitor esp32 --from-start
mcu-agentd monitor stm32 --from-start
```

## 日志与运行态目录

运行态目录固定为仓库根下的 `./.mcu-agentd/`（可删除；删除后需重新启动 daemon，并重新设置 selector）：

- `agentd.sock` / `agentd.lock`：单实例 IPC 与锁
- `selectors/<mcu_id>.txt`：selector 缓存（本仓库通过 `selector_cache_file` 复用 `.esp32-port/.stm32-port`）
- `sessions/<mcu_id>/*.session.ndjson`：一次性命令日志
- `monitor/<mcu_id>/*.mon.ndjson`：monitor 日志
- `meta/<mcu_id>.ndjson`：meta 事件日志

## 验收标准（迁移完成后）

- `mcu-agentd config validate` 在仓库根执行成功。
- `mcu-managerd projects add .` 注册成功，`mcu-agentd web` 可打开 Web UI，并能切换到 `ups120`。
- ESP32/STM32 的 selector 可写入 `.esp32-port`/`.stm32-port` 并被 `mcu-agentd selector get` 正确读回。
- `mcu-agentd flash/reset/monitor/logs` 对 `esp32` 与 `stm32` 均可用，且日志落盘到 `./.mcu-agentd/`。
