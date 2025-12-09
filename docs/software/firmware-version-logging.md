# UPS 固件版本信息启动日志设计

## 背景与目标

- 当前通过 `just ups-monitor` / `just sb-monitor` 查看启动日志时，无法一眼确认板子上跑的是哪一版固件，需要对照 ELF 或猜测。
- 本设计为两端固件（ESP32S3 `firmware/ups-main` 与 STM32L0 `firmware/smart-battery`）在启动早期统一打印一条包含版本信息的日志，便于排查「板子上跑的是哪一版」「刷写是否成功」等问题。
- 目标：在不改变现有 logging 后端、不引入显著开销的前提下，为每次上电 / 复位提供一次清晰的版本 / 构建信息输出。

## 范围与非目标

- 本次工作包含：
  - 为 `firmware/ups-main` 在 log 初始化完成后、主逻辑开始前增加一条 INFO 级版本日志；
  - 为 `firmware/smart-battery` 在 `defmt` 初始化完成后、主逻辑开始前增加一条 INFO 级版本日志；
  - 在两侧 crate 的构建流程中注入 git 短 hash 与（必要时）构建时间戳的环境变量，提供稳定的运行时常量。
- 不包含：
  - 不更改 ESP32 侧现有 `log` / `esp-println` / `esp-backtrace` 体系，也不更改 STM32 侧 `defmt` 后端；
  - 不新增协议级接口（例如通过串口命令查询版本），仅通过启动日志暴露信息；
  - 不修改 I2C、电源管理等业务逻辑。

## 版本信息字段设计

### 通用约定

- 版本信息由以下字段组成：
  - crate 名称：`"ups-main"` 或 `"smart-battery"`（硬编码常量）；
  - 语义版本：`env!("CARGO_PKG_VERSION")`，由 Cargo 自动提供；
  - git 短 hash：构建期通过 `build.rs` 或 CI 注入，失败时回退为 `"unknown"`；
  - 构建时间戳：`smart-battery` 已有 `SB_BUILD_TS`，`ups-main` 可选增加对称的 `UPS_BUILD_TS`。
- 建议统一日志文案格式：

```text
<crate>: version=<version> commit=<hash> build_ts=<ts>
```

其中：

- `<crate>`：`ups-main` / `smart-battery`；
- `<version>`：如 `0.1.0`；
- `<hash>`：如 `abcdef1`（`git rev-parse --short HEAD` 或 CI 注入截断值）；
- `<ts>`：ISO8601 或近似格式的构建时间字符串。

### 构建期环境变量

- 通用：
  - `CARGO_PKG_VERSION`：无需自定义，直接使用。
- `firmware/ups-main`：
  - 新增 `UPS_GIT_HASH`：git 短 hash 字符串；
  - 可选新增 `UPS_BUILD_TS`：构建时间戳字符串。
- `firmware/smart-battery`：
  - 已有 `SB_BUILD_TS`：构建时间戳；
  - 新增 `SB_GIT_HASH`：git 短 hash 字符串。

上述变量均以 `&'static str` 形式在运行时代码中通过 `env!` 宏展开，不引入动态分配或 `std` 依赖。

## 构建脚本设计

### 共同策略

- 在各自 crate 的 `build.rs` 中：
  - 优先尝试通过本地 git 获取短 hash：
    - 使用 `git rev-parse --short HEAD`；
    - 出错（无 git、非 git 仓库、浅克隆等）时不 panic，而是进入 fallback 分支。
  - 在 CI 环境下，若存在如 `GITHUB_SHA` 等变量：
    - 可读取并截断前 7 位作为短 hash；
    - 若变量缺失则继续 fallback。
  - 最终 fallback：将 hash 设为 `"unknown"`，防止构建失败。
- 通过 `println!("cargo:rustc-env=NAME=VALUE");` 将最终选择的 hash / build_ts 注入为编译期环境变量。

### `firmware/ups-main`

- 若当前 crate 尚无 `build.rs`：
  - 新增 `firmware/ups-main/build.rs`，实现上述逻辑：
    - 探测 git 短 hash → 设置 `UPS_GIT_HASH`；
    - 可选：生成或复用构建时间戳 → 设置 `UPS_BUILD_TS`。
- 若已有 `build.rs`：
  - 在现有脚本中扩展上述逻辑，保持原有功能不变。

### `firmware/smart-battery`

- 扩展现有 `build.rs`：
  - 在设置 `SB_BUILD_TS` 的同时，新增获取 git 短 hash 的逻辑；
  - 使用共同策略注入 `SB_GIT_HASH`；
  - 保证即便 git 或 CI 元信息不可用，`SB_GIT_HASH` 仍然会被设为 `"unknown"`。

## 运行时日志输出设计

### 输出时机与频率

- 对两侧固件都遵循以下原则：
  - 在启动流程早期输出版本日志，但要求：
    - logging 后端已经完成初始化；
    - 尚未进入主业务循环或高频日志阶段；
  - 每次上电 / 复位仅打印一次版本日志；
  - 不在定时任务或循环中重复输出，避免日志噪音。

### `firmware/ups-main`（ESP32S3）

- 在 `src/main.rs` 或实际入口模块中：
  - 找到 log / `esp-println` 初始化点（例如 logger 初始化或 `esp_println` 后备设定处）；
  - 在初始化完成后立即插入一次 INFO 级别日志。
- 运行时代码结构（示意）：

```rust
const VERSION: &str = env!("CARGO_PKG_VERSION");
const UPS_GIT_HASH: &str = env!("UPS_GIT_HASH");
const UPS_BUILD_TS: &str = env!("UPS_BUILD_TS");

// 在 logger 初始化完成后：
log::info!(
    "ups-main: version={} commit={} build_ts={}",
    VERSION,
    UPS_GIT_HASH,
    UPS_BUILD_TS,
);
```

- 实际使用的宏（`log::info!`、`esp_println` 包装等）需与现有工程保持一致，仅替换为适配的宏名称，不改变日志等级与输出后端。

### `firmware/smart-battery`（STM32L0）

- 在 `src/main.rs` 中：
  - 在 `defmt` logger 初始化完成之后、主任务或主循环开始之前插入一条版本日志；
  - 定义编译期常量：

```rust
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SB_GIT_HASH: &str = env!("SB_GIT_HASH");
const SB_BUILD_TS: &str = env!("SB_BUILD_TS");

defmt::info!(
    "smart-battery: version={} commit={} build_ts={}",
    VERSION,
    SB_GIT_HASH,
    SB_BUILD_TS,
);
```

- 由于该调用位于 main 初始化流程中，不在任何周期循环里，自然满足「每次上电 / 复位仅打印一次」的要求。

## 兼容性与失败模式

- 向后兼容：
  - 不修改对外接口和协议，仅新增日志，不影响现有上位机工具；
  - 旧固件不包含版本日志，新固件上线后即可通过日志区分。
- 失败模式：
  - 当 git 元信息不可用或命令失败时：
    - `*_GIT_HASH` 将为 `"unknown"`；
    - 日志仍然输出，其 commit 字段显示为 `unknown`；
    - 构建不会失败。
  - 构建时间戳生成失败时，可退回为固定字符串（例如 `"unknown"`），但推荐保持当前 `SB_BUILD_TS` 逻辑不变。

## 测试与验收计划

- 构建与刷写：
  - 在本地运行：
    - `just ups-build`；
    - `just sb-build`；
    - `just ups-flash`；
    - `just sb-flash`；
  - 确认上述命令均无新增警告或错误。
- 日志验证：
  - 通过 `just ups-monitor`：
    - 在启动早期看到类似 `ups-main: version=0.1.0 commit=abcdef1 build_ts=...`；
  - 通过 `just sb-monitor`：
    - 在启动早期看到类似 `smart-battery: version=0.1.0 commit=abcdef1 build_ts=...`；
  - 多次重刷 / 重启后，版本日志每次启动只出现一次；
  - 对比当前 git HEAD 短 hash 与日志中的 `commit` 字段，确认一致。
- 回归检查：
  - 监控启动日志，确认没有额外的高频输出或日志风暴；
  - 确认 I2C 通信、电源控制及其他既有功能行为不变；
  - 在 CI 环境执行现有构建流程，确保因缺少 git 元信息不会导致构建失败。

