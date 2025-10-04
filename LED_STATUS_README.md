# [已更新] 多灯指示规范已迁移

本文件为“单灯”原型说明，已被新版四色 LED 与状态机规范取代。请参考：`firmware/smart-battery/SOFTWARE_DESIGN.md`。

# LED状态指示实现（旧版，仅存档）

## 概述

本项目实现了一个LED状态指示系统，使用PA5引脚连接的LED来显示系统的不同状态。LED配置为开漏输出，低使能。

## 硬件配置

- **引脚**: PA5
- **输出类型**: 开漏输出 (Open Drain)
- **使能方式**: 低使能 (Active Low)
- **初始状态**: 高电平 (LED关闭)

## LED状态模式

按优先级从高到低排列，同一时间只显示一个匹配的状态：

### 1. 故障状态 (最高优先级)
- **触发条件**: 
  - BQ76920检测到故障：过压(OV)、欠压(UV)、短路放电(SCD)、过流放电(OCD)
  - SC8815检测到故障：过温保护(OTP)、VBUS短路故障
- **LED模式**: 4Hz频率闪烁
- **时序**: 125ms亮，125ms灭，周期250ms

### 2. 充电状态
- **触发条件**: SC8815检测到AC适配器连接且无故障
- **LED模式**: 0.5Hz频率闪烁
- **时序**: 1000ms亮，1000ms灭，周期2000ms

### 3. 充电完成状态
- **触发条件**: SC8815检测到充电结束(EOC)
- **LED模式**: 111011110节奏闪烁
- **时序**: 每位250ms，模式为[亮,亮,亮,灭,亮,亮,亮,亮,灭]

### 4. 放电状态 (暂未实现)
- **触发条件**: 系统处于放电模式
- **LED模式**: 10100000节奏闪烁
- **时序**: 每位250ms，模式为[亮,灭,亮,灭,灭,灭,灭,灭]

### 5. 正常状态 (最低优先级)
- **触发条件**: 无故障，无充电，无放电
- **LED模式**: LED关闭
- **状态**: 高电平

## 代码结构

### 主要文件

1. **`src/led_status_task.rs`**: LED状态指示任务实现
2. **`src/main.rs`**: 主程序，包含LED引脚配置和任务启动

### 关键函数

- `led_status_task()`: 主LED控制任务
- `evaluate_sc8815_status()`: 评估SC8815状态
- `evaluate_bq76920_status()`: 评估BQ76920状态
- `execute_pattern()`: 执行特定的闪烁模式

### 状态枚举

```rust
pub enum LedStatus {
    Fault,           // 故障状态 - 4Hz闪烁
    Charging,        // 充电状态 - 0.5Hz闪烁
    Discharging,     // 放电状态 - 10100000节奏闪烁
    ChargingComplete,// 充满状态 - 111011110节奏闪烁
    Normal,          // 正常状态 - LED关闭
}
```

## 数据流

1. **SC8815任务** → SC8815告警通道 → LED任务
2. **BQ76920任务** → BQ76920告警通道 → LED任务

LED任务订阅两个告警通道，实时监控系统状态并相应地控制LED。

## 配置说明

### GPIO配置
```rust
// 在main.rs中配置PA5为开漏输出
let led_pin = OutputOpenDrain::new(p.PA5, Level::High, Speed::Low);
```

### 任务启动
```rust
// 启动LED状态指示任务
spawner.spawn(led_status_task::led_status_task(
    led_pin,
    sc8815_alerts_channel.subscriber().unwrap(),
    bq76920_alerts_channel.subscriber().unwrap(),
)).unwrap();
```

## 注意事项

1. **优先级**: 故障状态具有最高优先级，会覆盖其他状态
2. **非阻塞**: LED任务使用非阻塞方式检查状态更新
3. **开漏输出**: 确保LED电路设计与开漏输出兼容
4. **低使能**: LED在低电平时点亮，高电平时熄灭

## 扩展功能

### 添加新状态
1. 在`LedStatus`枚举中添加新状态
2. 在主循环中添加状态检测逻辑
3. 在状态执行部分添加对应的LED控制逻辑

### 修改闪烁模式
修改`execute_pattern()`函数中的模式数组和时序参数。

## 调试

使用defmt日志查看状态变化：
```
INFO LED status changed to: Fault
INFO LED status changed to: Charging
```

## 测试建议

1. **故障测试**: 模拟过压/欠压条件，验证4Hz闪烁
2. **充电测试**: 连接AC适配器，验证0.5Hz闪烁
3. **充电完成测试**: 等待充电完成，验证111011110模式
4. **正常状态测试**: 断开充电器且无故障时，验证LED关闭
