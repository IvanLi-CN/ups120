# BQ25730 `read_adc_measurements` 函数优化计划

## 目标

优化 [`bq25730/src/lib.rs`](bq25730/src/lib.rs) 中的 `read_adc_measurements` 函数 ([`bq25730/src/lib.rs:272-309`](bq25730/src/lib.rs:272))，通过将多次单独的 ADC 寄存器读取操作合并为一次批量读取，以减少 I2C 通信次数，提高效率。

## 背景

当前 `read_adc_measurements` 函数通过多次调用 `self.read_registers()` 来分别读取各个 ADC 相关的寄存器。分析表明，这些 ADC 寄存器（从 `ADCPSYS` (0x26) 到 `ADCVSYS` (0x2D)）地址是连续的，共 8 个字节，适合进行批量读取。

## 详细计划

1.  **修改 `read_adc_measurements` 函数** ([`bq25730/src/lib.rs:272-309`](bq25730/src/lib.rs:272)):
    *   移除当前对 `ADCPSYS`, `ADCVBUS`, `ADCIDCHG`, `ADCICHG`, `ADCCMPIN`, `ADCIIN`, `ADCVBAT` 的多次单独 `self.read_registers()` 调用。
    *   替换为一次批量读取调用：
        ```rust
        let adc_data_raw = self.read_registers(Register::ADCPSYS, 8).await?;
        ```
    *   从 `adc_data_raw` (一个包含 8 个字节的 `heapless::Vec<u8, 30>`) 中解析出各个 ADC 值。字节与寄存器的对应关系如下：
        *   `adc_data_raw[0]`: `ADCPSYS`
        *   `adc_data_raw[1]`: `ADCVBUS`
        *   `adc_data_raw[2]`: `ADCIDCHG`
        *   `adc_data_raw[3]`: `ADCICHG`
        *   `adc_data_raw[4]`: `ADCCMPIN`
        *   `adc_data_raw[5]`: `ADCIIN`
        *   `adc_data_raw[6]`: `ADCVBAT` (LSB, 0x2C)
        *   `adc_data_raw[7]`: `ADCVSYS` (MSB of `ADCVBAT`, 0x2D, and also MSB for `ADCVSYS`)
    *   相应地更新 `AdcMeasurements` 结构体的初始化逻辑：
        ```rust
        Ok(AdcMeasurements {
            vbat: AdcVbat::from_register_value(adc_data_raw[6], adc_data_raw[7], offset_mv),
            vsys: AdcVsys::from_register_value(0, adc_data_raw[7], offset_mv), // LSB for VSYS ADC is not used from a separate reg
            ichg: AdcIchg::from_u8(adc_data_raw[3]),
            idchg: AdcIdchg::from_u8(adc_data_raw[2]),
            iin: AdcIin::from_u8(adc_data_raw[5], self.rsns_rac_is_5m_ohm),
            psys: AdcPsys::from_u8(adc_data_raw[0]),
            vbus: AdcVbus::from_u8(adc_data_raw[1]),
            cmpin: AdcCmpin::from_u8(adc_data_raw[4]),
        })
        ```
    *   `offset_mv` 的计算逻辑和 `self.rsns_rac_is_5m_ohm` 的使用保持不变。

2.  **确保根项目兼容性与构建成功**：
    *   `read_adc_measurements` 函数的外部接口（函数签名和返回类型）保持不变。
    *   在代码模式下完成对 [`bq25730/src/lib.rs`](bq25730/src/lib.rs) 的修改后，需要在根项目目录下运行 `cargo check` (以及可能的 `cargo build`) 来验证更改。
    *   如果 `cargo check` 报告任何错误或警告，将进行分析和修复，直至项目成功构建。

## 流程对比 Mermaid 图

```mermaid
graph TD
    A[开始 read_adc_measurements] --> B{计算 offset_mv};

    subgraph 当前实现
        B --> C1[读 ADCPSYS (1 byte)];
        C1 --> C2[读 ADCVBUS (1 byte)];
        C2 --> C3[读 ADCIDCHG (1 byte)];
        C3 --> C4[读 ADCICHG (1 byte)];
        C4 --> C5[读 ADCCMPIN (1 byte)];
        C5 --> C6[读 ADCIIN (1 byte)];
        C6 --> C7[读 ADCVBAT (2 bytes)];
        C7 --> D1[解析各个 ADC 值];
    end

    subgraph 优化后实现
        B --> E1[批量读取 ADC 寄存器 (0x26-0x2D, 8 bytes)];
        E1 --> F1[从批量数据中解析各个 ADC 值];
    end

    D1 --> G[构造 AdcMeasurements 结果];
    F1 --> G;
    G --> H[结束];
```

## 下一步
在用户确认此计划后，将请求切换到“代码”模式以实施这些更改。
