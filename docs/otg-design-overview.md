# OTG功能概要设计文档

## 1. 项目概述

### 1.1 功能描述
实现基于SC8815芯片的OTG（On-The-Go）功能，通过INA226电压检测实现智能电压控制：
- **设定电压**: Vs（用户可配置）
- **输出限流**: 1A
- **智能电压控制**:
  - INA226检测电压 > 90% Vs 时，输出 Vs - 0.5V
  - INA226检测电压 < 70% Vs 时，输出设定电压 Vs
  - 70%-90% Vs 区间采用滞回控制

### 1.2 系统集成
- **硬件平台**: RP2040微控制器
- **通信接口**: 独立I2C1总线（GP2/GP3）
- **软件框架**: Embassy异步任务系统
- **数据通信**: PubSub消息队列机制

## 2. 系统架构

### 2.1 整体架构
```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   INA226任务    │    │    OTG任务      │    │   LED状态任务   │
│  (电压检测)     │───▶│  (电压控制)     │───▶│  (状态指示)     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ INA226测量数据  │    │   OTG状态数据   │    │   USB数据上报   │
│   PubSub通道    │    │   PubSub通道    │    │      任务       │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

### 2.2 硬件连接
| 设备 | I2C总线 | 地址 | 引脚 | 功能 |
|------|---------|------|------|------|
| INA226 | I2C0 | 0x40 | GP0/GP1 | 电压检测 |
| SC8815_CHARGE | I2C0 | 0x6A | GP0/GP1 | 充电控制 |
| SC8815_OTG | I2C1 | 0x6A | GP2/GP3 | OTG控制 |
| BQ76920 | I2C0 | 0x08 | GP0/GP1 | 电池管理 |

### 2.3 任务架构
- **ina226_task**: 监测输入电压，发布测量数据
- **otg_task**: 订阅电压数据，控制SC8815 OTG输出
- **led_status_task**: 订阅OTG状态，更新LED指示
- **usb_task**: 订阅OTG状态，上报数据

## 3. 数据结构设计

### 3.1 OTG配置结构
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtgConfiguration {
    pub target_voltage_mv: u16,        // 设定电压 Vs (mV)
    pub high_threshold_percent: u8,     // 高阈值百分比 (默认90%)
    pub low_threshold_percent: u8,      // 低阈值百分比 (默认70%)
    pub voltage_reduction_mv: u16,      // 电压降低值 (默认500mV)
    pub current_limit_ma: u16,          // 输出限流 (默认1000mA)
    pub enabled: bool,                  // OTG功能使能
}
```

### 3.2 OTG状态结构
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtgStatus {
    pub enabled: bool,                  // OTG是否启用
    pub output_voltage_mv: u16,         // 当前输出电压
    pub output_current_ma: u16,         // 当前输出电流
    pub input_voltage_mv: u16,          // 检测到的输入电压
    pub control_state: OtgControlState, // 控制状态
    pub last_update_ms: u64,            // 最后更新时间
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OtgControlState {
    HighVoltage,    // 高电压状态 (>90% Vs)
    LowVoltage,     // 低电压状态 (<70% Vs)
    Normal,         // 正常状态 (70%-90% Vs)
    Disabled,       // 禁用状态
}
```

## 4. 控制逻辑设计

### 4.1 电压控制算法
```rust
fn calculate_target_voltage(
    input_voltage_v: f32,
    config: &OtgConfiguration,
    current_output_mv: u16,
) -> u16 {
    let vs_v = config.target_voltage_mv as f32 / 1000.0;
    let high_threshold_v = vs_v * (config.high_threshold_percent as f32 / 100.0);
    let low_threshold_v = vs_v * (config.low_threshold_percent as f32 / 100.0);
    
    if input_voltage_v > high_threshold_v {
        // 高电压：输出 Vs - 0.5V
        config.target_voltage_mv - config.voltage_reduction_mv
    } else if input_voltage_v < low_threshold_v {
        // 低电压：输出 Vs
        config.target_voltage_mv
    } else {
        // 滞回控制：保持当前输出
        current_output_mv
    }
}
```

### 4.2 滞回控制机制
- **目的**: 避免在阈值附近频繁切换
- **实现**: 在70%-90% Vs区间保持当前输出电压
- **稳定性**: 增加电压变化稳定性检测

## 5. LED状态集成

### 5.1 LED状态影响
OTG模块对LED状态的影响：
- **故障状态**: OTG故障导致系统进入`Fault`状态
- **放电状态**: 无市电时系统进入`Discharging`状态，OTG输出Vs
- **其他状态**: OTG工作但不影响LED显示

### 5.2 状态优先级
```
1. Fault           // 包含OTG故障
2. Charging        // 充电中（OTG输出Vs-0.5V）
3. ChargingComplete // 充满电（OTG输出Vs-0.5V）
4. Discharging     // 放电中（OTG输出Vs）
5. SystemActive    // 正常运行（OTG可能工作）
6. Initializing    // 初始化中
7. Normal          // 空闲状态
```

## 6. 通信协议

### 6.1 PubSub通道设计
```rust
// OTG状态 PubSub
const OTG_STATUS_PUBSUB_DEPTH: usize = 4;
const OTG_STATUS_PUBSUB_READERS: usize = 2; // usb_task + led_status_task

pub type OtgStatusPublisher<'a> = Publisher<'a, ...>;
pub type OtgStatusSubscriber<'a> = Subscriber<'a, ...>;
```

### 6.2 USB数据协议扩展
```rust
pub struct AllMeasurementsUsbPayload {
    // ... 现有字段 ...
    
    // OTG相关字段
    pub otg_enabled: u8,
    pub otg_output_voltage_mv: u16,
    pub otg_output_current_ma: u16,
    pub otg_input_voltage_mv: u16,
    pub otg_control_state: u8,
}
```

## 7. 错误处理

### 7.1 故障检测
- **I2C通信超时**: 5秒无响应
- **输出电压异常**: 与设定值偏差过大
- **输出电流过载**: 超过1.2A（1A + 20%容差）
- **设备初始化失败**: 启动时配置失败

### 7.2 故障恢复
- **自动重试**: I2C通信失败时自动重试
- **安全关闭**: 严重故障时停止OTG输出
- **状态上报**: 通过PubSub通道上报故障状态

## 8. 性能指标

### 8.1 响应性能
- **电压检测周期**: 1秒
- **控制响应时间**: < 2秒
- **状态更新频率**: 1Hz

### 8.2 精度要求
- **电压控制精度**: ±50mV
- **电流限制精度**: ±100mA
- **阈值检测精度**: ±1%

## 9. 安全考虑

### 9.1 保护机制
- **过载保护**: 输出电流限制1A
- **短路保护**: SC8815内置短路保护
- **过温保护**: 设备温度监控
- **通信看门狗**: 定期检查设备响应

### 9.2 故障安全
- **默认安全状态**: 故障时停止输出
- **渐进启动**: 启动时逐步增加输出
- **状态监控**: 实时监控所有关键参数

## 10. 测试策略

### 10.1 单元测试
- 电压控制算法测试
- 滞回控制逻辑测试
- 故障检测机制测试

### 10.2 集成测试
- I2C通信测试
- PubSub消息传递测试
- LED状态集成测试

### 10.3 系统测试
- 长时间稳定性测试
- 负载变化响应测试
- 故障恢复测试
