# SC8815 外部电阻配置指南

## 概述

本项目已配置为使用外部电阻分压器来设置SC8815的充电电压，而不是使用内部寄存器设置。这种配置方式提供了更精确和稳定的电压控制。

## 配置变更

在 `src/sc8815_task.rs` 中，关键的配置变更如下：

```rust
// 使用外部电阻配置模式 - 只需要这一行配置！
config.battery.use_internal_setting = false;
```

## 重要说明

### cell_count 和 voltage_per_cell 在外部模式下完全无效！

**根据SC8815官方文档明确说明：**

> "If VBAT_SEL is set to 1, it means the battery voltage is set externally. Under this condition, the user should use resistor divider at VBATS pin to set the target voltage as below. **VCELL_SET and CSEL bits don't work.**"

这意味着：

1. **完全无效**：在外部电阻模式下，`cell_count` 和 `voltage_per_cell` 参数被SC8815硬件完全忽略
2. **不影响任何功能**：既不影响充电电压，也不影响ADC比例设置
3. **可以设置为任意值**：这些参数在外部模式下没有任何作用

### 正确的外部电阻模式配置

```rust
// 外部电阻模式只需要这一行配置：
config.battery.use_internal_setting = false;
// 其他battery参数在外部模式下被硬件完全忽略，无需设置
```

## 外部电阻计算

### 基本公式

```
VBAT = VBATREF_E × (1 + R_UP/R_DOWN)
```

其中：
- `VBATREF_E`: 内部参考电压（默认约1V，可编程范围0.7V-2.048V）
- `R_UP`: 从VBATS引脚到高电压轨的电阻
- `R_DOWN`: 从VBATS引脚到地的电阻

### 常见配置示例

#### 4S锂电池 (16.8V充电电压)

**方案1：使用默认参考电压**
- 目标电压：16.8V
- VBATREF_E：1.0V（默认）
- 所需比例：16.8 = 1.0 × (1 + R_UP/R_DOWN)
- R_UP/R_DOWN = 15.8
- 推荐值：R_UP = 158kΩ, R_DOWN = 10kΩ

**方案2：调整参考电压**
- 目标电压：16.8V
- 选择R_UP = 130kΩ, R_DOWN = 10kΩ
- 比例：1 + 130/10 = 14
- 所需VBATREF_E：16.8V / 14 = 1.2V

#### 3S锂电池 (12.6V充电电压)

- 目标电压：12.6V
- VBATREF_E：1.0V（默认）
- 所需比例：12.6 = 1.0 × (1 + R_UP/R_DOWN)
- R_UP/R_DOWN = 11.6
- 推荐值：R_UP = 116kΩ, R_DOWN = 10kΩ

## 硬件连接

### 电阻连接
```
VBAT+ ----[R_UP]---- VBATS ----[R_DOWN]---- GND
                        |
                    SC8815芯片
```

### 推荐电阻规格
- **阻值范围**：
  - R_UP: 100kΩ - 200kΩ
  - R_DOWN: 10kΩ - 20kΩ
- **精度**：1%或更高
- **功率**：1/4W足够（电流很小）
- **温度系数**：低温度系数电阻（如金属膜电阻）

## 调整参考电压（可选）

如果需要微调充电电压，可以通过编程调整内部参考电压VBATREF_E：

```rust
// 在SC8815配置中添加参考电压设置
// 注意：这需要在SC8815库中实现相应的函数
// sc8815.set_vbat_reference_voltage(1200).await?; // 设置为1.2V
```

## 验证配置

### 1. 检查ADC读数
启动系统后，检查SC8815的ADC读数：
```
[SC8815] VBUS:12000mV, VBAT:16800mV, IBUS:0mA, IBAT:0mA
```

### 2. 用万用表测量
- 测量VBATS引脚电压（应该约为VBATREF_E值）
- 测量实际充电电压
- 验证电阻分压比例

## 注意事项

1. **安全第一**：在连接电池前，务必验证充电电压设置正确
2. **电阻精度**：使用高精度电阻确保电压准确性
3. **温度影响**：考虑温度对电阻值的影响
4. **PCB布局**：电阻应靠近SC8815芯片放置，减少干扰
5. **备用方案**：保留切换回内部电压设置的能力

## 故障排除

### 充电电压不正确
1. 检查电阻值和连接
2. 验证VBATS引脚电压
3. 检查SC8815配置（use_internal_setting = false）

### ADC读数异常
1. 确认voltage_per_cell设置用于ADC计算
2. 检查VBAT监控比例设置

### 无法充电
1. 验证PSTOP引脚控制
   - [2025-11-25 订正] 以净表与 `SOFTWARE_DESIGN.md` 为准，区分三个信号：
     - `PSTOP_MCU`：MCU 侧 GPIO（本项目为 `PA9`），由固件直接驱动；
     - `PSTOP_CTL`：硬件保护逻辑输出，`PSTOP_CTL = TEMP_FAULT_N · PSTOP_MCU`；
     - `PSTOP`：SC8815 芯片引脚，经板载反相器由 `PSTOP_CTL` 翻转而来。
   - 有效语义为：
     - `PSTOP_CTL = 1` → 反相后 `PSTOP = Low` → **功率级允许工作**；
     - `PSTOP_CTL = 0` → 反相后 `PSTOP = High` → **功率级停机**；
     - 固件只控制 `PSTOP_MCU`，拉低它一定会使 `PSTOP_CTL=0`（无论温度是否正常）从而停机；拉高时，仅在 `TEMP_FAULT_N=1`（温度正常）时功率级才放行。
   - 旧文档中“`PSTOP_OUT 高=停机`、`低=允许`”的说法已过时且与当前网表不符，应忽略，以本节订正说明为准。
2. 检查电流限制设置
3. 确认充电模式已启用
