# BatteryConfig 结构体修改计划

## 目标

在 `BatteryConfig` 结构体中添加检流电阻的阻值和过流/短路检测阈值相关的配置。

## 详细计划

1.  **修改 `BatteryConfig` 结构体：**
    *   在 [`src/main.rs`](src/main.rs) 中找到 `BatteryConfig` 结构体的定义。
    *   添加一个新的字段来存储检流电阻的阻值，例如命名为 `shunt_resistance_mohms`，类型为 `u32`。
    *   添加新的字段来存储过流 (OCD) 和短路 (SCD) 检测阈值的寄存器值。根据 BQ76920 的数据手册，这些阈值通常配置在 `PROTECT1`, `PROTECT2`, `PROTECT3` 寄存器中。我们可以添加字段来直接存储这些寄存器的值，例如 `protect1_reg_val`, `protect2_reg_val`, `protect3_reg_val`，类型为 `u8`。
    *   更新 `BatteryConfig` 的实例化代码，包含新添加的字段并赋予合适的初始值。

2.  **更新使用 `BatteryConfig` 的代码：**
    *   查找代码中所有使用 `BatteryConfig` 结构体的地方。
    *   根据需要调整相关逻辑，例如在配置 BQ76920 芯片时，使用 `shunt_resistance_mohms` 来计算或验证阈值寄存器值，并将 `protect1_reg_val`, `protect2_reg_val`, `protect3_reg_val` 写入相应的寄存器。

## Mermaid 图示

```mermaid
graph TD
    A[用户需求：在 BatteryConfig 中添加检流电阻和过流/短路阈值配置] --> B{分析现有代码};
    B --> C[读取 src/main.rs 中的 BatteryConfig 定义];
    C --> D[确定需要添加的字段];
    D --> E[制定修改计划];
    E --> F[修改 BatteryConfig 结构体定义];
    E --> G[更新 BatteryConfig 实例化代码];
    E --> H[更新使用 BatteryConfig 的相关代码];
    F --> I[完成结构体修改];
    G --> J[完成实例化修改];
    H --> K[完成相关代码更新];
    I & J & K --> L[计划完成，准备实施];