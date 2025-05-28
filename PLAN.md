# 项目代码调整计划

## 背景
用户更新了项目依赖，导致编译失败，并报告了 `no method named to_register_value found for struct AdcPsys in the current scope` 等错误。经过分析，确认 `bq25730-async-rs` 库中的 `Adc*` 结构体的 `to_register_value()` 方法已在上游代码变更中移除。此外，还存在一些未使用的导入和未读取变量的警告。

## 目标
1.  解决 `src/shared.rs` 中 `BinWrite` 实现的编译错误。
2.  清理 `src/shared.rs` 中未使用的导入。
3.  解决 `src/main.rs` 中未读取变量的警告。

## 详细修复计划

### 1. 修改 `src/shared.rs` 中的导入语句
*   在 `src/shared.rs` 中，添加对 `bq25730_async_rs::data_types` 中所有 `Adc*` 结构体（`AdcPsys`, `AdcVbus`, `AdcIdchg`, `AdcIchg`, `AdcCmpin`, `AdcIin`, `AdcVbat`, `AdcVsys`）的导入，以便能够访问它们的 `LSB_MV` 或 `LSB_MA` 常量。
*   移除未使用的 `CoulombCounter` 导入。

### 2. 修改 `src/shared.rs` 中的 `BinWrite` 实现
*   对于 `impl<const N: usize> BinWrite for AllMeasurements<N>` 块中的每个 `Adc*` 结构体，将 `self.bq25730.adc_measurements.xxx.to_register_value()` 替换为 `(self.bq25730.adc_measurements.xxx.0 / AdcXXX::LSB_YY) as u8`。
*   `LSB_YY` 将根据具体的 `Adc*` 类型是电压还是电流而定，分别是 `LSB_MV` 或 `LSB_MA`。

    **示例（以 `psys` 为例）：**
    *   **旧代码**：
        ```rust
        self.bq25730
            .adc_measurements
            .psys
            .to_register_value()
            .write_options(writer, endian, args)?;
        ```
    *   **新代码**：
        ```rust
        (self.bq25730.adc_measurements.psys.0 / AdcPsys::LSB_MV) as u8
            .write_options(writer, endian, args)?;
        ```

### 3. 解决 `src/main.rs` 中的未读取变量警告
*   检查 `src/main.rs` 的相关代码，如果这些变量（`voltages`, `temps`, `current`, `system_status`, `mos_status`, `bq25730_measurements`）确实没有被使用，将它们重命名为 `_variable_name` 来消除警告。

## 代码结构变化示意图

```mermaid
graph TD
    A[src/shared.rs] --> B{BinWrite impl for AllMeasurements}
    B --> C{Old: .to_register_value()}
    B --> D{New: .0 / AdcXXX::LSB_YY as u8}
    A --> E[Imports]
    E --> F{Old: AdcMeasurements}
    E --> G{New: AdcMeasurements, AdcPsys, AdcVbus, AdcIdchg, AdcIchg, AdcCmpin, AdcIin, AdcVbat, AdcVsys}
