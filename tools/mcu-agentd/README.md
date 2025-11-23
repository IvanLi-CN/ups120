# mcu-agentd

单实例守护进程，用统一命令流为两路 MCU（ESP32-S3、STM32L0）提供固件烧录、复位、端口缓存、后台日志采集与查询。CLI 与守护同一可执行文件，默认后台监控自动开启，所有命令返回本机时间戳便于对齐日志。

## 目录结构
- `src/` Rust 实现（tokio + clap）。
- `logs/agentd/` 运行期产物：
  - `agentd.sock` / `agentd.lock`：单实例通信与锁。
  - `esp32.meta.log` / `stm32.meta.log`：事件元数据（NDJSON）。
  - `esp32/`、`stm32/`：会话/监控日志（`*-mon.log`、`*.session.log`）。
- 端口缓存文件：仓库根 `.esp32-port`、`.stm32-port`（兼容读取 `.stm32-probe`）。

## 核心行为
- 守护单例：`start` 启动后台；`stop` 关闭；`status` 返回 PID、sock、当前时间。
- 端口缓存：`set-port` / `get-port` 读写上述端口文件。
- 固件操作会自动暂停后台监控以释放串口，完成后恢复监控：
  - ESP32 烧录：`espflash flash <elf> --chip esp32s3 --port <cache> --after no-reset`
  - STM32 烧录：`probe-rs download --chip STM32L051C8Tx --probe <cache> <elf>`
  - 复位：`espflash reset ...` / `probe-rs reset ...`
- 后台监控（守护启动且端口存在时自动开启）：
  - ESP32：`espflash monitor --non-interactive --elf <default or user> --log-format defmt`（会 reset 并运行）
  - STM32：`probe-rs run --log-format oneline <elf>`（运行固件，不停核）
  - 监控输出写入 `*-mon.log`，元事件写入 `*.meta.log`。
- monitor 命令：直接 tail 最新会话/监控日志，**实时逐行输出原文，不回放历史**；默认无限跟随，支持 `--duration`、`--lines` 限制。
- logs 命令：从元数据文件筛选事件，支持 `--since/--until`（RFC3339）、`--tail`、`--sessions`（附每个 session 尾部行）。
- 默认 ELF：ESP32 `firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main`，STM32 `target/thumbv6m-none-eabi/release/smart-battery`；如不存在直接报错（不再自动构建）。

## 命令速查（完整路径示例）
```bash
# 守护
./target/debug/mcu-agentd start
./target/debug/mcu-agentd status
./target/debug/mcu-agentd stop

# 端口缓存
./target/debug/mcu-agentd set-port --mcu esp32 /dev/cu.usbmodem412201
./target/debug/mcu-agentd get-port --mcu stm32

# 固件操作（自动停/恢复监控）
./target/debug/mcu-agentd flash --mcu esp32 --elf <path> --after no-reset
./target/debug/mcu-agentd flash --mcu stm32 --elf <path>
./target/debug/mcu-agentd reset --mcu esp32
./target/debug/mcu-agentd reset --mcu stm32

# 实时日志（只看新增，不回放）
./target/debug/mcu-agentd monitor esp32               # 无限跟随，Ctrl+C 退出
./target/debug/mcu-agentd monitor stm32 --duration 10s --lines 50

# 日志查询（元数据 + 可选 session 尾部）
./target/debug/mcu-agentd logs --mcu all --tail 50 --sessions
./target/debug/mcu-agentd logs --mcu esp32 --since 2025-11-23T10:00:00+08:00 --until 2025-11-23T11:00:00+08:00
```

## 日志格式
- 元数据（NDJSON，每行一个事件）：
  ```json
  {"ts":"2025-11-23T18:55:29.718+08:00","mono_ms":15666,"mcu":"esp32","event":"flash","status":0,"duration_ms":1784,"session":".../20251123_124106.session.log"}
  ```
- 会话/监控（行原样或包装 JSON）：
  - 监控行示例：`{"ts":"2025-11-23T19:06:37.012+08:00","mcu":"esp32","src":"stdout","text":"680914 ms [INFO ] smart-battery temps => ..."}`
  - monitor 命令会提取 `text` 直接打印原文；若非 JSON 行则原样打印。

## 运行与依赖
- 需要已安装 `espflash`, `probe-rs`（probe-rs-cli），并确保默认 ELF 已构建。
- 仓库根使用 Makefile，勿在仓根直接 cargo build/flash firmware；mcu-agentd 为独立二进制可用 `cargo build -p mcu-agentd`。

## 已知未实现 / 待办
- 未提供配置文件覆盖（`configs/mcu-agentd.toml`）。
- 不再自动构建缺省 ELF；需手动构建后再运行命令。
- 日志查询基于文件扫描，未引入 SQLite 索引。

