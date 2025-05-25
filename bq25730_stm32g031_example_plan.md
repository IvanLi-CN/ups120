# 为 BQ25730 驱动创建 STM32G031C8U6 示例项目计划

### 目标

为 `bq25730` 驱动创建一个独立的 `stm32g031c8u6` 示例项目，该项目将初始化 STM32G031C8U6 的 I2C 外设，并使用 `bq25730-async-rs` 库读取 BQ25730 芯片的状态和测量数据。

### 计划步骤

1.  **创建项目目录**: 在当前工作目录 `/Volumes/ExData/Projects/Ivan/ups120` 下创建一个新的目录，命名为 `bq25730_stm32g031_example`。
2.  **初始化 Cargo 项目**: 在 `bq25730_stm32g031_example` 目录中运行 `cargo init --bin`，创建一个新的 Rust 二进制项目。
3.  **更新 `bq25730_stm32g031_example/Cargo.toml`**:
    *   修改 `package` 部分，确保 `name` 和 `edition` 正确。
    *   添加以下依赖：
        *   `embassy-stm32`
        *   `embassy-embedded-hal`
        *   `embassy-executor`
        *   `embassy-time`
        *   `defmt`
        *   `defmt-rtt`
        *   `panic-probe`
        *   `static_cell`
        *   `heapless`
        *   `bq25730-async-rs` (作为本地路径依赖，指向 `../bq25730`)
    *   配置 `[features]`，启用 `defmt` 和 `async`。
    *   添加 `[profile.dev]` 和 `[profile.release]` 配置，以优化嵌入式开发。
4.  **创建 `bq25730_stm32g031_example/.cargo/config.toml`**: 配置 Cargo runner，以便使用 `probe-run` 等工具进行烧录和调试。
5.  **编写 `bq25730_stm32g031_example/src/main.rs`**:
    *   复制 `src/main.rs` 中 I2C 初始化和 `bq25730` 相关的代码作为基础。
    *   调整 `use` 语句以适应新的项目结构和依赖。
    *   简化 `main` 函数，只保留 `bq25730` 的初始化和数据读取逻辑。
    *   添加 `bq25730.init().await?` 调用，执行 BQ25730 的基本初始化。
    *   在主循环中，定期读取 BQ25730 的各种状态和测量寄存器（例如 `ChargerStatus`, `ProchotStatus`, `AdcMeasurements`, `ChargeCurrent`, `ChargeVoltage` 等），并使用 `defmt::info!` 打印其值。
6.  **提供运行示例的说明**: 告知用户如何编译、烧录和运行这个示例项目。

### 流程图

```mermaid
graph TD
    A[开始] --> B{创建 bq25730_stm32g031_example 目录};
    B --> C[初始化 Cargo 项目];
    C --> D[更新 Cargo.toml];
    D --> E[创建 .cargo/config.toml];
    E --> F[编写 src/main.rs];
    F --> G[实现 BQ25730 初始化和数据读取循环];
    G --> H[提供运行示例说明];
    H --> I[完成];