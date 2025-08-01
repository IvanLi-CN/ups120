# OTG功能需求分析文档 - LED指示灯补充

## LED指示灯需求分析补充

### 1. 现有LED状态系统分析

#### 1.1 当前LED状态定义
基于`src/led_status_task.rs`的分析，现有LED状态系统已实现：

| 状态 | 模式 | 频率 | 描述 | 优先级 |
|------|------|------|------|--------|
| **Fault** | 快速闪烁 | 4Hz (125ms开/关) | 关键系统故障或通信错误 | 最高 |
| **Charging** | 慢速闪烁 | 0.5Hz (1000ms开/关) | 电池充电中 | 高 |
| **ChargingComplete** | 模式闪烁 | 111011110模式 | 电池充满 | 高 |
| **Discharging** | 模式闪烁 | 10100000模式 | 电池放电 | 中 |
| **SystemActive** | 心跳闪烁 | 1Hz (100ms开, 900ms关) | 正常运行，所有设备响应 | 中 |
| **Initializing** | 中速闪烁 | 2Hz (250ms开/关) | 系统启动和设备初始化 | 低 |
| **Normal** | 关闭 | - | 系统空闲 | 最低 |

#### 1.2 状态评估逻辑
```rust
// 当前状态优先级（从高到低）
1. Fault                     // 任何故障（BQ76920/SC8815故障、通信超时）
2. Charging/ChargingComplete // 充电相关状态
3. Discharging              // 放电状态（暂未实现）
4. SystemActive             // 系统正常运行
5. Initializing             // 初始化状态
6. Normal                   // 空闲状态
```

### 2. OTG LED状态扩展设计（修正版）

#### 2.1 OTG在LED状态中的正确定位
根据需求澄清，OTG模块在LED指示灯中只有两种相关状态：

1. **放电状态**: UPS使用电池放电，市电掉线，OTG输出额定电压
2. **故障状态**: OTG模块发生故障

**重要原则**:
- OTG模块只在"放电"模式下才在LED中体现
- 如果OTG没有故障，且不在放电模式，则OTG不影响LED状态
- OTG的电压调节逻辑（高电压/低电压模式）不在LED中体现

#### 2.2 修正后的状态设计
OTG相关的LED状态变化：

| 系统状态 | LED状态 | 说明 |
|----------|---------|------|
| **市电正常，在充电** | `Charging` | 现有状态，OTG不工作 |
| **市电正常，充满电** | `ChargingComplete` | 现有状态，OTG不工作 |
| **市电掉线，电池放电** | `Discharging` | 现有状态，**OTG在此时工作** |
| **OTG故障** | `Fault` | 现有故障状态，包含OTG故障 |
| **系统正常运行** | `SystemActive` | 现有状态，OTG可能工作但不体现 |

#### 2.3 状态优先级（保持现有不变）
```rust
// 状态优先级（从高到低）- 无需修改
1. Fault                     // 包含OTG故障
2. Charging / ChargingComplete // 充电相关状态
3. Discharging               // 电池放电状态（OTG工作）
4. SystemActive              // 系统正常运行
5. Initializing              // 初始化状态
6. Normal                    // 空闲状态
```

#### 2.3 OTG对现有LED状态的影响

##### 2.3.1 Discharging状态的增强
现有的`Discharging`状态（10100000模式）将承担OTG工作时的指示：
- **触发条件**: 市电掉线，电池放电，OTG输出电压
- **LED模式**: 保持现有的10100000模式不变
- **工作逻辑**: OTG在后台根据INA226电压调节输出，但LED不体现调节过程

##### 2.3.2 Fault状态的扩展
现有的`Fault`状态将包含OTG故障：
- **OTG故障检测**: I2C通信失败、输出异常、过载保护等
- **LED模式**: 保持现有的4Hz快速闪烁不变
- **故障优先级**: OTG故障与其他系统故障同等优先级

### 3. LED状态评估逻辑修正

#### 3.1 简化的数据源需求
LED任务需要能够检测OTG故障，但不需要订阅详细的OTG状态：

```rust
// 方案1: 通过现有的故障检测机制
// LED任务继续使用现有的订阅，OTG故障通过系统故障通道报告

// 方案2: 最小化的OTG状态订阅（仅故障信息）
pub async fn led_status_task(
    // ... 现有参数保持不变 ...
    mut otg_status_subscriber: OtgStatusSubscriber<'static>, // 可选，仅用于故障检测
)
```

#### 3.2 简化的状态评估逻辑
```rust
fn determine_system_status_with_otg(
    // ... 现有参数 ...
    otg_status: &Option<OtgStatus>, // 可选参数
) -> LedStatus {
    // 1. 检查故障状态（最高优先级）
    if has_system_fault() || has_otg_fault(otg_status) {
        return LedStatus::Fault; // 统一使用现有Fault状态
    }

    // 2. 检查充电状态（高优先级）
    if is_charging() {
        return LedStatus::Charging;
    }
    if is_charging_complete() {
        return LedStatus::ChargingComplete;
    }

    // 3. 检查放电状态（OTG可能在此时工作，但LED不区分）
    if is_discharging() {
        return LedStatus::Discharging; // OTG在后台工作，LED显示放电
    }

    // 4. 其他状态检查...
    if all_devices_healthy() {
        return LedStatus::SystemActive;
    }

    // ... 其他现有状态逻辑保持不变
}
```

#### 3.3 OTG故障检测逻辑
```rust
fn has_otg_fault(otg_status: &OtgStatus) -> bool {
    // 检查OTG相关故障条件
    let now = embassy_time::Instant::now();
    let last_update = Duration::from_millis(otg_status.last_update_ms);
    
    // 通信超时检测
    if now.duration_since_epoch() - last_update > Duration::from_secs(5) {
        return true;
    }
    
    // 输出电压异常检测
    if otg_status.output_voltage_mv == 0 && otg_status.enabled {
        return true;
    }
    
    // 输出电流过载检测
    if otg_status.output_current_ma > 1200 { // 1A限流 + 20%容差
        return true;
    }
    
    false
}
```

### 4. 实现细节（简化版）

#### 4.1 LED状态枚举（无需修改）
现有的LED状态枚举完全满足需求，无需添加新状态：

```rust
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum LedStatus {
    Initializing,     // 系统初始化
    Fault,           // 故障状态（包含OTG故障）
    Charging,        // 充电中
    Discharging,     // 放电中（OTG在此时工作）
    ChargingComplete, // 充电完成
    SystemActive,    // 系统正常运行
    Normal,          // 空闲状态
}
```

#### 4.2 现有模式保持不变
无需添加新的LED模式，现有模式完全适用：

```rust
match current_status {
    LedStatus::Fault => {
        // 4Hz快速闪烁 - 包含OTG故障
        if now.duration_since(last_update) >= Duration::from_millis(125) {
            led.toggle();
            last_update = now;
        }
    }
    LedStatus::Discharging => {
        // 10100000模式 - OTG在此时工作但LED不体现调节过程
        execute_pattern(
            &mut led,
            &mut pattern_index,
            &mut last_update,
            &[true, false, true, false, false, false, false, false],
            Duration::from_millis(250),
        );
    }
    // ... 其他现有模式保持不变
}
```

### 5. 调试和监控（简化版）

#### 5.1 最小化的调试信息扩展
```rust
// 可选：在5秒调试输出中包含OTG故障状态
info!(
    "LED Debug - Status: {:?}, SC8815_init: {}, BQ76920_init: {}, OTG_fault: {}",
    new_status, sc8815_initialized, bq76920_initialized,
    otg_status.map(|s| has_otg_fault(s)).unwrap_or(false)
);
```

#### 5.2 简化的状态变化日志
```rust
// 保持现有的状态变化日志，OTG相关信息在Discharging和Fault状态中体现
if new_status != current_status {
    match new_status {
        LedStatus::Discharging => info!("System discharging - OTG may be active"),
        LedStatus::Fault => {
            if has_otg_fault(&otg_status) {
                warn!("System fault detected - including OTG fault");
            } else {
                warn!("System fault detected");
            }
        }
        _ => info!("LED status changed to: {:?}", new_status),
    }
}
```

### 6. 向后兼容性（完全兼容）

#### 6.1 零修改原则
- **LED状态枚举**: 完全不需要修改
- **LED模式**: 所有现有模式保持不变
- **优先级逻辑**: 现有优先级完全适用
- **调试输出**: 最小化修改，保持兼容

#### 6.2 实现策略
1. **阶段1**: 仅在故障检测中添加OTG故障检查
2. **阶段2**: 在Discharging状态触发条件中考虑OTG工作状态
3. **阶段3**: 可选的调试信息增强

### 7. 总结

#### 7.1 LED指示灯的OTG集成原则
- **简化原则**: OTG不增加新的LED状态，复用现有状态
- **透明原则**: OTG的电压调节逻辑对LED完全透明
- **故障优先**: OTG故障通过现有Fault状态体现
- **放电指示**: OTG工作时通过现有Discharging状态体现

#### 7.2 关键设计决策
1. **不添加新LED状态**: 避免复杂化用户体验
2. **复用现有模式**: 保持系统一致性
3. **最小化修改**: 确保向后兼容
4. **故障统一处理**: 简化故障诊断

这个修正版的LED分析确保了OTG功能与现有LED系统的完美集成，遵循了"OTG只在放电模式体现，其他时候透明"的设计原则。
