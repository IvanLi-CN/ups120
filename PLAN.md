# BQ76920 充电问题修复方案与实施计划

## 1. 问题概述

设备当前无法充电。根本原因在于 `BQ76920` 电池管理芯片的充电MOS管使能位 (`CHG_ON` 在 `SYS_CTRL2` 寄存器中) 在初始化后未被主动设置为 `true`。这导致 `bq25730_task` 在检查 `BQ76920` 状态时，认为充电不被允许，进而设置 `BQ25730` 充电芯片的 `CHRG_INHIBIT` 位，禁止了充电操作。

## 2. 核心修复思路

在 `BQ76920` 初始化过程中，首先应用所有安全相关的配置参数（如过压、欠压、过流保护阈值等）。然后，严格验证这些关键配置是否已成功写入芯片。只有在配置验证通过的前提下，才主动使能 `BQ76920` 的充电MOS管 (`CHG_ON`) 和放电MOS管 (`DSG_ON`)，从而默认允许充电和放电通路。后续的实际充放电控制将依赖 `BQ76920` 芯片自身的硬件保护机制以及 `bq25730_task` 根据 `BQ76920` 报告的状态进行的判断。

此方案将配置验证逻辑封装到 `bq769x0_async_rs` 驱动库中，通过一个名为 `try_apply_config` 的新方法实现。

## 3. 详细实施步骤

### 阶段一：修改 `bq769x0_async_rs` 驱动库 ([`bq76920/`](bq76920/))

1.  **更新 `bq76920/src/errors.rs`**:
    *   在 `Error` 枚举中添加新的错误变体，用于表示配置验证失败：
        ```rust
        ConfigVerificationFailed {
            register: registers::Register, // 确保 Register 类型可被包含
            expected: u8,
            actual: u8,
        }
        ```

2.  **在 `bq76920/src/lib.rs` 中实现 `async fn try_apply_config()`**:
    *   定义新的公共方法 `pub async fn try_apply_config(&mut self, config: &BatteryConfig) -> Result<(), Error<E>>`。
    *   **内部实现**:
        *   **调用现有 `set_config`**: 首先调用 `self.set_config(config).await?` 执行实际的寄存器写入。
        *   **实现配置验证逻辑**:
            *   读取ADC校准值 (`adc_gain_uv_per_lsb`, `adc_offset_mv`)。
            *   回读关键安全配置寄存器：`OV_TRIP`, `UV_TRIP`, `PROTECT1`, `PROTECT2`, `PROTECT3`, `CC_CFG`, `SYS_CTRL1`, `SYS_CTRL2` (基础配置部分，此时 `CHG_ON`/`DSG_ON` 应为0)。
            *   根据传入的 `config` 和ADC校准值，计算这些寄存器的期望原始值。
            *   逐个比较回读值与期望值。若任何关键配置不匹配，则返回 `Err(Error::ConfigVerificationFailed { register, expected, actual })`。
        *   如果所有验证都通过，则返回 `Ok(())`。

### 阶段二：修改应用层代码 ([`src/bq76920_task.rs`](src/bq76920_task.rs:1))

 1.  **在 `bq76920_task` 的初始化部分**:
     *   替换原有的 `bq.set_config(...)` 调用为 `bq.try_apply_config(&battery_config).await`。
     *   **错误处理**:
         *   若 `try_apply_config` 返回 `Ok(_)`:
             *   记录配置成功和已验证的日志。
             *   继续调用 `bq.enable_charging().await` 和 `bq.enable_discharging().await` 来使能MOS管。
             *   记录尝试使能MOS管的日志，并处理这两个调用可能产生的I2C错误。
         *   若 `try_apply_config` 返回 `Err(Error::ConfigVerificationFailed {..})`):
             *   记录包含详细不匹配信息的**严重错误**日志。
             *   **任务不应继续使能MOS管**，并应考虑进入安全停机状态或持续报错，以防止在保护配置不正确的情况下运行。
         *   若 `try_apply_config` 返回其他错误 (如 `Error::I2c(_)`):
             *   记录错误，并同样考虑采取安全措施。
     *   移除在 `bq76920_task.rs` 中对 `bq.write_register(Register::CcCfg, 0x19).await` 和 `bq.clear_status_flags(0xFF).await` 的直接调用，因为这些操作已包含在驱动库的 `set_config` (并因此在 `try_apply_config`) 内部。

## 4. 预期效果

完成上述修改后，`BQ76920` 将在初始化时进行严格的配置验证。只有在关键安全参数确认无误后，才会默认开启充电和放电通路。这将使得 `BQ76920` 的 `CHG_ON` 和 `DSG_ON` 标志被正确设置为 `true`（前提是配置验证通过且使能命令成功），从而允许 `BQ25730` 根据其自身的逻辑以及从 `BQ76920` 获取的状态信息来控制充电过程。`BQ76920` 芯片自身的硬件保护机制在此基础上依然有效，提供底层的安全保障。

```mermaid
graph TD
    A[系统初始化] --> B(启动 bq25730_task)
    A --> C(启动 ina2226_task)
    A --> D(启动 bq76920_task)
    B --> B1{bq25730_task.rs 逻辑}
    C --> C1{ina226_task.rs 逻辑}
    D --> D1{bq76920_task.rs 逻辑}
    B1 --> E[发布 Bq25730 数据/警报]
    C1 --> F[发布 Ina226 数据]
    D1 --> G[发布 Bq76920 数据/警报]
    E --> H(Pub/Sub)
    F --> H
    G --> H
    H --> I[其他任务 (例如 USB)]

# USB 通信功能排查与修复计划

**目标：** 定位并解决上位机无法从 `device` 固件通过 USB 正常订阅和接收数据的问题。

**背景分析回顾：**
根据对固件 ([`device/src/main.rs`](device/src/main.rs:1), [`device/src/usb/mod.rs`](device/src/usb/mod.rs:1), [`device/src/usb/endpoints.rs`](device/src/usb/endpoints.rs:1)) 的代码审查，固件具备以下 USB 通信能力：
-   **命令接收与处理：**
    -   能够接收上位机发送的命令，特别是 `SubscribeStatus` (magic byte `0x00`) 和 `UnsubscribeStatus` (magic byte `0x01`)。
    -   `process_command` 函数 ([`device/src/usb/endpoints.rs:88`](device/src/usb/endpoints.rs:88)) 负责处理这些命令并更新内部的 `status_subscription_active` ([`device/src/usb/endpoints.rs:33`](device/src/usb/endpoints.rs:33)) 状态。
-   **响应发送：**
    -   当收到 `SubscribeStatus` 命令后，固件会通过其响应端点 (`response_write_ep` - [`device/src/usb/endpoints.rs:29`](device/src/usb/endpoints.rs:29)) 发送一个 `StatusResponse` ([`device/src/usb/endpoints.rs:20`](device/src/usb/endpoints.rs:20))。此响应以 magic byte `0x80` 开头，并包含当前的 `AllMeasurements` ([`device/src/data_types.rs:49`](device/src/data_types.rs:49)) 数据。
-   **数据推送：**
    -   当 `status_subscription_active` ([`device/src/usb/endpoints.rs:33`](device/src/usb/endpoints.rs:33)) 为 `true` 时，固件会通过其推送端点 (`push_write_ep` - [`device/src/usb/endpoints.rs:30`](device/src/usb/endpoints.rs:30)) 定期发送 `StatusPush` ([`device/src/usb/endpoints.rs:24`](device/src/usb/endpoints.rs:24)) 数据。此推送数据以 magic byte `0xC0` 开头，并包含 `AllMeasurements` ([`device/src/data_types.rs:49`](device/src/data_types.rs:49)) 数据。

**排查与修复步骤：**

1.  **验证上位机正确发送 `SubscribeStatus` 命令：**
    *   **操作：** 检查上位机代码，确保其向固件的命令端点（`command_read_ep` - [`device/src/usb/endpoints.rs:28`](device/src/usb/endpoints.rs:28) 在固件侧）发送了正确的 `SubscribeStatus` 命令。
    *   **验证：** 命令应为单个字节 `0x00`。
    *   **工具：** 可以使用 USB 分析工具（如 Wireshark 与 USBPcap，或特定平台的 USB 嗅探工具）捕获 USB 流量，确认该字节已发送。
    *   **固件侧日志：** 固件在 `usb_task` ([`device/src/usb/mod.rs:124`](device/src/usb/mod.rs:124)) 中有 `defmt::info!("USB command received: {:?}", cmd);` 日志，可以确认是否收到了命令。

2.  **验证上位机正确接收并解析 `StatusResponse`：**
    *   **操作：** 检查上位机代码，确保其在发送 `SubscribeStatus` 命令后，能够从固件的响应端点接收数据。
    *   **验证：** 上位机应能接收到以 `0x80` 开头的数据包，并能根据 `AllMeasurements` ([`device/src/data_types.rs:49`](device/src/data_types.rs:49)) 的结构正确解析后续数据。
    *   **工具：** USB 分析工具。
    *   **固件侧日志：** 固件在 `send_response` ([`device/src/usb/endpoints.rs:74`](device/src/usb/endpoints.rs:74)) 中有 `defmt::info!("固件发送响应原始字节: {:x}", &self.write_buffer[..len]);` 日志。

3.  **验证上位机正确接收并解析 `StatusPush` 数据：**
    *   **操作：** 检查上位机代码，确保其能够从固件的推送端点接收数据。
    *   **验证：** 上位机应能接收到以 `0xC0` 开头的数据包，并能根据 `AllMeasurements` ([`device/src/data_types.rs:49`](device/src/data_types.rs:49)) 的结构正确解析后续数据。
    *   **工具：** USB 分析工具。
    *   **固件侧日志：** 固件在 `send_status_update` ([`device/src/usb/endpoints.rs:125`](device/src/usb/endpoints.rs:125)) 中有 `defmt::info!("固件发送原始字节: {:x}", &self.write_buffer[..len]);` 日志。

4.  **数据结构一致性检查 (`AllMeasurements`)：**
    *   **操作：** 仔细比对上位机用于解析 `AllMeasurements` ([`device/src/data_types.rs:49`](device/src/data_types.rs:49)) 的数据结构定义与固件中 `binrw` 序列化/反序列化的行为。
    *   **注意：** 确保字段顺序、类型、大小端等均一致。`binrw` 默认使用大端序 (Big Endian)。

5.  **USB 端点配置与能力检查：**
    *   **操作：** 确认上位机期望的 USB 端点类型（Interrupt, Bulk等）、方向、最大包大小等配置与固件在 `UsbEndpoints::new` ([`device/src/usb/endpoints.rs:37`](device/src/usb/endpoints.rs:37)) 中的配置一致。固件配置的是 Interrupt 端点，最大包大小为 64 字节。

6.  **使用 USB 分析工具进行端到端流量分析：**
    *   **操作：** 捕获从上位机发送命令到固件响应和推送数据的完整 USB 交互过程。
    *   **分析：** 检查是否有 USB 协议层面的错误、数据包是否完整、端点是否按预期工作。

**固件调试增强（可选，如果上述步骤未能定位问题）：**

1.  **增加更详细的 USB 事件日志：**
    *   在固件的 `usb_task` ([`device/src/usb/mod.rs:35`](device/src/usb/mod.rs:35)) 和 `UsbEndpoints` ([`device/src/usb/endpoints.rs:27`](device/src/usb/endpoints.rs:27)) 的关键路径（如端点读写前后、状态变更时）添加更详细的 `defmt` 日志，以便更精确地追踪执行流程和数据状态。

**预期成果：**
*   明确上位机无法接收到 USB 数据的根本原因。
*   如果问题在上位机，提供明确的修改建议。
*   如果问题在固件（尽管目前分析可能性较低），定位到具体代码并进行修复。
*   最终目标是使上位机能够成功订阅并持续接收到固件推送的 `AllMeasurements` 数据。

**通信流程示意图：**

```mermaid
sequenceDiagram
    participant 上位机
    participant 固件 (device)

    上位机->>+固件: 发送 SubscribeStatus 命令 (0x00) 到 command_read_ep
    Note over 固件: process_command() -> status_subscription_active = true
    固件->>-上位机: 发送 StatusResponse (0x80 + AllMeasurements) 到 response_write_ep

    loop 周期性数据推送 (当 status_subscription_active 为 true)
        Note over 固件: usb_task 聚合数据
        固件->>上位机: 发送 StatusPush (0xC0 + AllMeasurements) 到 push_write_ep
    end

    Note right of 上位机: 上位机持续监听 push_write_ep 以接收数据

    alt 用户取消订阅
        上位机->>+固件: 发送 UnsubscribeStatus 命令 (0x01) 到 command_read_ep
        Note over 固件: process_command() -> status_subscription_active = false
        固件-->>-上位机: (可选) 发送确认响应
    end
```

---
