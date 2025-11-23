# MCU Agent 服务设计说明

## 目的与范围

- 单实例服务，统一管理 ESP32-S3 与 STM32L0 的固件烧录、复位、日志采集、端口缓存。
- 面向自动化/Agent 调用，每次操作需返回本地时间戳、耗时与结果，避免日志丢失。

## 设计概览

- 语言/框架：Rust（tokio + clap）。
- 工程位置：`tools/mcu-agentd/`（独立二进制 crate，隔离于固件工程）。
- 实例模型：同一可执行文件兼具守护与客户端，通过 Unix socket + 锁文件保证单实例；per-MCU 资源锁避免占用串口/探针冲突。
- 端口缓存：仓根 `.esp32-port`、`.stm32-port`（兼容读取 `.stm32-probe`）；提供 set/get/list 命令。
- 运行策略：烧录/复位默认不自动运行；仅在日志采集（monitor/attach）时启动目标。若底层命令必须复位，日志里显式标注事件。
- 日志目录：`logs/agentd/` 下的元数据与会话日志；支持滚动与时间范围查询。

## 指令面（当前实现）

- `start | stop | status`：管理守护，返回 PID、sock 路径、当前时间。
- `set-port --mcu {esp32,stm32} --path PATH` / `get-port --mcu ...`（兼容读取 `.stm32-probe`）。
- `flash --mcu esp32 --elf PATH [--after no-reset|hard-reset]`
- `flash --mcu stm32 --elf PATH`
- `reset --mcu esp32|stm32`（操作期间内部自动暂停/恢复后台监控，避免串口占用冲突）。
- `monitor --mcu {esp32,stm32} [--duration 30s] [--lines N]`：仅从最新会话日志尾部分流显示，不执行底层命令。
- `logs --mcu {esp32,stm32,all} [--since RFC3339] [--until RFC3339] [--tail N] [--sessions]`：可选择附带每个 session 的尾部行。

说明：
- 后台监控在守护启动且端口存在时自动起；用户层不再暴露 start/stop 监控命令，所有占用串口的操作会自动停/恢复监控。
- 目前不再提供“构建兜底”，`--elf` 缺省时若默认 ELF 不存在会直接报错，需先自行构建。

## 底层命令选择

- ESP32-S3：
  - 烧录：`espflash flash <elf> --chip esp32s3 --port <cache> --after no-reset`
  - 后台监控：`espflash monitor --chip esp32s3 --port <cache> --no-reset --non-interactive --elf <elf> --log-format defmt`
  - 复位：`espflash reset --chip esp32s3 --port <cache>`
- STM32L0：
  - 烧录：`probe-rs download --chip STM32L051C8Tx --probe <cache> <elf>`（不复位）
  - 后台监控：`probe-rs attach --chip STM32L051C8Tx --probe <cache> --log-format oneline <elf>`
  - 复位：`probe-rs reset --chip STM32L051C8Tx --probe <cache>`

## 日志格式与查询

- 时间戳：统一 RFC3339（含时区）字段 `ts`；并记录单调时间 `mono_ms`（进程启动以来毫秒）以抵抗系统时钟跳变。
- 元数据日志（NDJSON）：一行一事件，示例：  
  `{"ts":"2025-11-23T14:05:31.842-08:00","mono_ms":124422,"mcu":"esp32","event":"flash","elf":".../ups-main","port":"/dev/cu.SLAB_USBtoUART","status":"ok","code":0,"duration_ms":8123,"op_id":"op-20251123-1405-esp32-1"}`
  - 文件：`logs/agentd/{esp32,stm32}.meta.log`（按大小/天数滚动，可配置）。
- 会话日志（NDJSON，每行为封装后的原始输出）：
  - 后台监控行示例：`{"ts":"2025-11-23T17:51:00.645+08:00","mcu":"esp32","src":"stderr","text":"[... Using flash stub]"}`
  - 命令执行行示例：`{"ts":"2025-11-23T12:41:06.702+08:00","mcu":"esp32","event":"log"} <raw line>`
  - 文件：`logs/agentd/{mcu}/YYYYMMDD_HHMMSS-mon.log`（后台监控），`YYYYMMDD_HHMMSS.session.log`（一次性命令会话）。
- 查询流程：先按文件名日期粗过滤，再按 `ts` 精过滤；命令 `logs --mcu ... [--since/--until RFC3339] [--tail N]` 实现。
- 心跳：守护每 60s 写入心跳元数据（持锁状态、活跃会话）用于异常检测。
- 可选索引：默认仅文件；如需高效检索，可启用内置 SQLite 索引（单文件 `logs/agentd/meta.db`，索引 `ts,mcu,event`，记录会话文件与偏移），不改变现有文件格式。

## 验收与进展

| 功能 | 描述 | 验收标准 | 状态 | 备注 |
| --- | --- | --- | --- | --- |
| 单实例守护 | 锁文件+Unix socket；start/stop/status 可用 | 第二实例报 already running；status 显示 PID | 已实现 |  |
| 端口缓存管理 | 读写 `.esp32-port`、`.stm32-port`，兼容 `.stm32-probe` | set/get 正确读写 | 已实现 |  |
| 烧录 ESP32 | flash 默认 `--after no-reset` | 返回 ts/耗时，设备不自动运行 | 已实现 |  |
| 烧录 STM32 | probe-rs download 不复位 | 返回 ts/耗时，设备不自动运行 | 已实现 |  |
| 复位控制 | 分 MCU 调用 espflash/probe-rs reset | 复位事件入元数据，附时间戳 | 已实现 |  |
| 日志采集 | 后台监控捕获输出 + session/元数据；monitor 仅尾随显示 | logs 可按 MCU/时间过滤；tail N 生效 | 已实现（未引入 SQLite 索引） |  |
| 构建兜底 | 缺 ELF 自动 make 对应目标 | 默认 ELF 不存在即报错；不自动构建 | 已移除 | 需手动先构建 |
| 配置文件 | `configs/mcu-agentd.toml` 覆盖默认 | 配置生效，env 可覆盖 | 未实现 |  |

## 非目标

- 不处理跨机远程烧录；不内置 OTA 流程。
- 不修改固件日志格式，仅透传 defmt/串口输出。

## 后续迭代建议

- 增加 HTTP/gRPC 接口以便外部系统调用。
- 自动检测端口热插拔并刷新缓存。
- 可选“安全模式”：烧录前自动备份旧固件（如 ESP32 读取 flash）。
