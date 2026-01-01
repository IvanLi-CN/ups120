# mcu-agentd 迁移（ups120）

## 背景

`ups120` 仓库历史上维护过一套项目内自研的 `ups120-agentd`（位于 `tools/` 下的 `mcu-agentd` 子项目），用于统一执行 ESP32/STM32 的烧录、复位与日志采集。该实现与外部项目 `../mcu-agentd` 的能力与维护节奏发生分叉，导致重复维护与行为不一致风险。

本工作项将 `ups120` 的 on-target 工作流切换到外部 `mcu-agentd`/`mcu-managerd`，并移除仓库内的自研实现。

## 目标

- 统一入口：`ups120` 的 flash/reset/monitor/logs/selector 统一通过外部 `mcu-agentd` 执行。
- 保留现有端口/探针缓存习惯：ESP32 使用 `.esp32-port`，STM32 使用 `.stm32-port`。
- 使用 Web UI：通过 `mcu-agentd` 启动时自动注册项目并提供 Web UI/HTTP API 入口。
- 移除遗留实现与垃圾内容：删除 `tools/mcu-agentd/`，并删除旧版 STM32 probe selector 缓存文件（不再生成/读取）。

## 非目标

- 不修改固件业务逻辑与通信协议。
- 不引入远程烧录/OTA 流程。
- 不对底层工具（`espflash`/`probe-rs`）做参数透传增强；行为通过 `mcu-agentd.toml` 约束。

## 范围

### in-scope

- 新增仓库根 `mcu-agentd.toml`，将 MCU 定义抽象为两个 `mcu_id`：
  - `esp32`：backend=`espflash`，chip=`esp32s3`
  - `stm32`：backend=`probe-rs`，chip=`STM32L051C8Tx`
- 迁移 `just` 包装：
  - 保留 `sb-*` 与 `ups-*` 命令作为主入口。
  - `agentd-*` 命令语义与外部工具对齐（selector/web/config validate 等）。
- Web UI 工作流落地：`open "$(mcu-agentd web)"`
- 清理遗留：
  - 删除 `tools/mcu-agentd/`
  - 删除旧版 STM32 probe selector 缓存文件，并确保脚本/文档不再引用它

### out-of-scope

- 新增第三路 MCU 或更多 target。
- 修改 `mcu-agentd` 外部项目的实现。

## 用例与流程

### 1) 首次使用（一次性）

1. 全局安装外部工具并启动 daemon：
   - `just agentd-init`
2. 在仓库根创建并校验 `mcu-agentd.toml`：
   - `mcu-agentd config validate`
3. 打开 Web UI：
   - `open "$(mcu-agentd web)"`
4. 设置 selector（建议显式指定 VALUE，避免交互模式的严格校验门槛）：
   - ESP32：`mcu-agentd selector set esp32 /dev/cu.usbmodemXXXX`
   - STM32：`mcu-agentd selector set stm32 0483:3748:SERIAL`

### 2) 日常使用（开发）

- ESP32：
  - `just ups-build`
  - `just ups-flash`
  - `mcu-agentd monitor esp32 --from-start`
  - `mcu-agentd logs esp32 --tail 200 --sessions`
- STM32：
  - `just sb-build`
  - `just sb-flash`
  - `mcu-agentd monitor stm32 --from-start`
  - `mcu-agentd logs stm32 --tail 200 --sessions`

## 数据与运行态目录（state）

外部 `mcu-agentd` 的运行态目录固定为 `${project_root}/.mcu-agentd/`：

- `agentd.sock` / `agentd.lock`
- `sessions/<mcu_id>/*.session.ndjson`
- `monitor/<mcu_id>/*.mon.ndjson`
- `meta/<mcu_id>.ndjson`

端口/探针缓存文件由配置 `selector_cache_file` 显式指向仓库根：

- `.esp32-port`
- `.stm32-port`

约束：

- 旧版 STM32 probe selector 缓存文件必须删除且不再生成/读取。

## 接口与模块边界（概要设计）

### 1) 配置契约（`mcu-agentd.toml`）

- `project.id` 固定为 `ups120`（用于 `mcu-managerd` 注册与切换）。
- MCU 定义以 `mcu_id` 为唯一键：
  - `esp32` / `stm32`
- `artifact_elf` 路径必须与本仓库构建产物一致；若未来调整构建输出目录（例如自定义 `CARGO_TARGET_DIR`），必须同步更新配置。

### 2) `just` 作为唯一入口

- `just` 仍是团队约定的唯一入口；但其内部不再依赖 `cargo run --release` 启动仓库内的 agentd crate。
- `just` 的职责：
  - 负责调用固件 build；
  - 负责调用 `mcu-agentd` 进行 on-target 操作；
  - 不直接调用底层 `espflash`/`probe-rs`。

### 3) selector 缓存策略

- 以 `.esp32-port` 与 `.stm32-port` 为唯一真相文件。
- `scripts/*probe*`（若仍保留）必须遵循该策略，禁止生成旧版 STM32 probe selector 缓存文件。

## 兼容性与迁移

- 从 `logs/agentd/` 迁移到 `./.mcu-agentd/`：
  - 新旧日志路径不同，排障文档与使用说明必须更新。
- 命令语义迁移：
  - `monitor` 不再支持 `--duration/--lines`；由使用者通过 Ctrl+C 或外部 shell 控制。
- selector 文件迁移：
  - 旧版 STM32 probe selector 缓存文件不再兼容；仅 `.stm32-port` 生效。

## 风险与缓解

- 旧 daemon 占用串口/探针导致失败：迁移步骤应明确 `mcu-agentd stop` 并确认无遗留进程。
- `artifact_elf` 路径不一致导致 flash 失败：在验收中强制执行 `mcu-agentd config validate`，并在 README/文档强调构建顺序。
- selector 交互模式受严格校验限制：文档与 `just` 建议使用显式 `selector set <mcu_id> <value>`。

## 验收标准

- 配置与注册：
  - `mcu-agentd config validate` 在仓库根执行成功。
  - `mcu-agentd start` 启动成功，`open "$(mcu-agentd web)"` 可打开 Web UI 并切换到 `ups120`。
- 缓存文件：
  - `.esp32-port` / `.stm32-port` 可被 `mcu-agentd selector get` 正确读回。
  - 旧版 STM32 probe selector 缓存文件不存在，且仓库内无脚本/文档再引用它。
- 日常动作：
  - `mcu-agentd flash/reset/monitor/logs` 对 `esp32` 与 `stm32` 均可用，且日志落盘到 `./.mcu-agentd/`。
- 清理：
  - `tools/mcu-agentd/` 不存在，`Justfile` 不再引用它。
