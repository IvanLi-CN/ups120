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
