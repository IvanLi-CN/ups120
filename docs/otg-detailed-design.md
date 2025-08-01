# OTG功能详细设计文档

## 1. 模块详细设计

### 1.1 数据类型定义 (src/data_types.rs)

#### 1.1.1 OTG配置结构
```rust
/// OTG配置参数
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub struct OtgConfiguration {
    /// 设定电压 Vs (mV)
    pub target_voltage_mv: u16,
    /// 高阈值百分比 (默认90%)
    pub high_threshold_percent: u8,
    /// 低阈值百分比 (默认70%)
    pub low_threshold_percent: u8,
    /// 电压降低值 (默认500mV)
    pub voltage_reduction_mv: u16,
    /// 输出限流 (默认1000mA)
    pub current_limit_ma: u16,
    /// OTG功能使能
    pub enabled: bool,
}

impl Default for OtgConfiguration {
    fn default() -> Self {
        Self {
            target_voltage_mv: 12000,      // 12V
            high_threshold_percent: 90,
            low_threshold_percent: 70,
            voltage_reduction_mv: 500,     // 0.5V
            current_limit_ma: 1000,       // 1A
            enabled: true,
        }
    }
}
```

#### 1.1.2 OTG状态结构
```rust
/// OTG运行状态
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub struct OtgStatus {
    /// OTG是否启用
    pub enabled: bool,
    /// 当前输出电压 (mV)
    pub output_voltage_mv: u16,
    /// 当前输出电流 (mA)
    pub output_current_ma: u16,
    /// 检测到的输入电压 (mV)
    pub input_voltage_mv: u16,
    /// 控制状态
    pub control_state: OtgControlState,
    /// 最后更新时间戳 (ms)
    pub last_update_ms: u64,
}

impl Default for OtgStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            output_voltage_mv: 0,
            output_current_ma: 0,
            input_voltage_mv: 0,
            control_state: OtgControlState::Disabled,
            last_update_ms: 0,
        }
    }
}

/// OTG控制状态枚举
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum OtgControlState {
    /// 高电压状态 (>90% Vs) - 输出 Vs-0.5V
    HighVoltage,
    /// 低电压状态 (<70% Vs) - 输出 Vs
    LowVoltage,
    /// 正常状态 (70%-90% Vs) - 滞回控制
    Normal,
    /// 禁用状态
    Disabled,
}
```

#### 1.1.3 USB数据协议扩展
```rust
/// USB传输数据结构扩展
impl AllMeasurementsUsbPayload {
    // 在现有结构中添加OTG字段
    pub otg_enabled: u8,                // OTG使能状态
    pub otg_output_voltage_mv: u16,     // OTG输出电压
    pub otg_output_current_ma: u16,     // OTG输出电流
    pub otg_input_voltage_mv: u16,      // OTG检测输入电压
    pub otg_control_state: u8,          // OTG控制状态
}
```

### 1.2 共享资源扩展 (src/shared.rs)

#### 1.2.1 PubSub通道定义
```rust
// OTG状态 PubSub
const OTG_STATUS_PUBSUB_DEPTH: usize = 4;
const OTG_STATUS_PUBSUB_READERS: usize = 2; // usb_task + led_status_task
static OTG_STATUS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        OtgStatus,
        OTG_STATUS_PUBSUB_DEPTH,
        OTG_STATUS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// 类型别名
pub type OtgStatusPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    OtgStatus,
    OTG_STATUS_PUBSUB_DEPTH,
    OTG_STATUS_PUBSUB_READERS,
    1,
>;

pub type OtgStatusSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    OtgStatus,
    OTG_STATUS_PUBSUB_DEPTH,
    OTG_STATUS_PUBSUB_READERS,
    1,
>;

pub type OtgStatusChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    OtgStatus,
    OTG_STATUS_PUBSUB_DEPTH,
    OTG_STATUS_PUBSUB_READERS,
    1,
>;
```

#### 1.2.2 初始化函数扩展
```rust
// 扩展PubSubSetup类型
pub type PubSubSetup<'a, const N: usize> = (
    // ... 现有字段 ...
    OtgStatusPublisher<'a>,
    &'a OtgStatusChannelType,
);

// 扩展初始化函数
pub fn init_pubsubs() -> PubSubSetup<'static, 5> {
    // ... 现有初始化 ...
    
    let otg_status_pubsub: &'static OtgStatusChannelType =
        OTG_STATUS_PUBSUB.init(PubSubChannel::new());
    
    (
        // ... 现有返回值 ...
        otg_status_pubsub.publisher().unwrap(),
        otg_status_pubsub,
    )
}
```

## 2. OTG任务实现 (src/otg_task.rs)

### 2.1 任务签名
```rust
/// OTG任务 - 使用SC8815实现OTG功能
#[embassy_executor::task]
pub async fn otg_task(
    i2c_bus: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_rp::peripherals::I2C1, embassy_rp::i2c::Async>,
    >,
    address: u8,
    config: OtgConfiguration,
    mut ina226_measurements_subscriber: Ina226MeasurementsSubscriber<'static>,
    otg_status_publisher: OtgStatusPublisher<'static>,
) {
    info!("OTG task started - SC8815 OTG mode with voltage control");
    info!("OTG Config: Vs={}mV, High={}%, Low={}%", 
          config.target_voltage_mv, config.high_threshold_percent, config.low_threshold_percent);
    
    // 任务实现...
}
```

### 2.2 初始化逻辑
```rust
// 创建SC8815驱动实例
let mut sc8815 = SC8815::new(i2c_bus, address);

// 初始化SC8815为OTG模式
if let Err(e) = sc8815.init().await {
    error!("Failed to initialize SC8815 for OTG: {:?}", e);
    return;
}

// 配置OTG模式
let mut device_config = DeviceConfiguration::default();
device_config.power.operating_mode = OperatingMode::Discharging; // OTG模式
device_config.current_limits.ibus_limit_ma = config.current_limit_ma;
device_config.power.vinreg_voltage_mv = config.target_voltage_mv;

if let Err(e) = sc8815.configure_device(&device_config).await {
    error!("Failed to configure SC8815 for OTG: {:?}", e);
    return;
}

info!("SC8815 OTG mode configured successfully");
```

### 2.3 主控制循环
```rust
// 控制状态变量
let mut current_output_voltage_mv = config.target_voltage_mv - config.voltage_reduction_mv;
let mut last_voltage_v = 0.0;
let mut voltage_stable_count = 0;

// 计算阈值
let vs_v = config.target_voltage_mv as f32 / 1000.0;
let high_threshold_v = vs_v * (config.high_threshold_percent as f32 / 100.0);
let low_threshold_v = vs_v * (config.low_threshold_percent as f32 / 100.0);
let reduced_voltage_v = vs_v - (config.voltage_reduction_mv as f32 / 1000.0);

loop {
    // 获取INA226测量数据
    let ina226_measurements = ina226_measurements_subscriber.next_message_pure().await;
    let input_voltage_v = ina226_measurements.voltage;
    
    // 检查电压变化稳定性
    let voltage_changed = (input_voltage_v - last_voltage_v).abs() > 0.1;
    if voltage_changed {
        voltage_stable_count = 0;
    } else {
        voltage_stable_count = voltage_stable_count.saturating_add(1);
    }
    last_voltage_v = input_voltage_v;
    
    // 确定目标输出电压和控制状态
    let (target_output_voltage_mv, control_state) = if input_voltage_v > high_threshold_v {
        // 高电压：输出 Vs - 0.5V
        ((reduced_voltage_v * 1000.0) as u16, OtgControlState::HighVoltage)
    } else if input_voltage_v < low_threshold_v {
        // 低电压：输出 Vs
        (config.target_voltage_mv, OtgControlState::LowVoltage)
    } else {
        // 滞回控制：保持当前输出
        (current_output_voltage_mv, OtgControlState::Normal)
    };
    
    // 更新输出电压（如果需要且电压稳定）
    if target_output_voltage_mv != current_output_voltage_mv && voltage_stable_count >= 3 {
        if let Err(e) = sc8815.set_vbus_voltage_mv(target_output_voltage_mv).await {
            error!("Failed to set OTG output voltage: {:?}", e);
            // 发布故障状态
            publish_fault_status(&otg_status_publisher, &config).await;
            continue;
        }
        
        current_output_voltage_mv = target_output_voltage_mv;
        info!("OTG voltage updated: {}mV (state: {:?})", 
              current_output_voltage_mv, control_state);
    }
    
    // 读取当前输出状态
    let (output_current_ma, actual_voltage_mv) = match read_otg_status(&mut sc8815).await {
        Ok(status) => status,
        Err(e) => {
            error!("Failed to read OTG status: {:?}", e);
            publish_fault_status(&otg_status_publisher, &config).await;
            continue;
        }
    };
    
    // 发布OTG状态
    let otg_status = OtgStatus {
        enabled: config.enabled,
        output_voltage_mv: actual_voltage_mv,
        output_current_ma,
        input_voltage_mv: (input_voltage_v * 1000.0) as u16,
        control_state,
        last_update_ms: embassy_time::Instant::now().as_millis(),
    };
    
    otg_status_publisher.publish_immediate(otg_status);
    
    // 等待下一个周期
    Timer::after(Duration::from_secs(1)).await;
}
```

### 2.4 辅助函数
```rust
/// 读取OTG状态
async fn read_otg_status(sc8815: &mut SC8815<I2cDevice<...>>) -> Result<(u16, u16), Error> {
    let measurements = sc8815.get_adc_measurements().await?;
    Ok((measurements.ibus_ma, measurements.vbus_mv))
}

/// 发布故障状态
async fn publish_fault_status(
    publisher: &OtgStatusPublisher<'static>,
    config: &OtgConfiguration,
) {
    let fault_status = OtgStatus {
        enabled: false,
        output_voltage_mv: 0,
        output_current_ma: 0,
        input_voltage_mv: 0,
        control_state: OtgControlState::Disabled,
        last_update_ms: embassy_time::Instant::now().as_millis(),
    };
    publisher.publish_immediate(fault_status);
}
```

## 3. 主程序集成 (src/main.rs)

### 3.1 I2C1总线配置
```rust
// 配置I2C1总线用于OTG
let i2c1_sda = p.PIN_2;
let i2c1_scl = p.PIN_3;
let i2c1 = I2c::new_async(p.I2C1, i2c1_scl, i2c1_sda, Irqs, i2c::Config::default());
let i2c1_bus_mutex = Mutex::<CriticalSectionRawMutex, _>::new(i2c1);
let i2c1_bus_mutex = make_static!(i2c1_bus_mutex);
```

### 3.2 OTG任务启动
```rust
// 创建OTG I2C设备
let otg_i2c_device = I2cDevice::new(i2c1_bus_mutex);

// OTG配置
let otg_config = OtgConfiguration::default();
let otg_address = 0x6A; // SC8815地址

// 创建订阅者
let ina226_measurements_subscriber_for_otg = ina226_measurements_channel.subscriber().unwrap();

// 启动OTG任务
spawner
    .spawn(otg_task::otg_task(
        otg_i2c_device,
        otg_address,
        otg_config,
        ina226_measurements_subscriber_for_otg,
        otg_status_publisher,
    ))
    .unwrap();

info!("OTG task spawned");
```

### 3.3 引脚重新分配
```rust
// 重新分配GPIO引脚
let pstop_pin = Output::new(p.PIN_4, Level::High);  // 移动到GP4
let discharge_pin = Input::new(p.PIN_5, Pull::Up);  // 移动到GP5
let charge_pin = Input::new(p.PIN_6, Pull::Up);     // 移动到GP6
// GP2/GP3 现在用于I2C1
```

## 4. LED状态集成

### 4.1 LED任务扩展
```rust
// 在led_status_task中添加OTG状态订阅
pub async fn led_status_task(
    led_pin: Output<'static>,
    // ... 现有参数 ...
    mut otg_status_subscriber: OtgStatusSubscriber<'static>,
) {
    // ... 现有逻辑 ...
    
    // 检查OTG状态更新
    if let Some(embassy_sync::pubsub::WaitResult::Message(otg_status)) =
        otg_status_subscriber.try_next_message()
    {
        latest_otg_status = Some(otg_status);
        if !otg_status.enabled {
            // OTG故障检测
            if has_otg_fault(&otg_status) {
                return LedStatus::Fault;
            }
        }
    }
}

/// 检查OTG故障
fn has_otg_fault(otg_status: &OtgStatus) -> bool {
    let now = embassy_time::Instant::now();
    let last_update = Duration::from_millis(otg_status.last_update_ms);
    
    // 通信超时检测
    if now.duration_since_epoch() - last_update > Duration::from_secs(5) {
        return true;
    }
    
    // 输出异常检测
    if otg_status.output_voltage_mv == 0 && otg_status.enabled {
        return true;
    }
    
    // 过载检测
    if otg_status.output_current_ma > 1200 { // 1A + 20%容差
        return true;
    }
    
    false
}
```

## 5. USB数据协议集成

### 5.1 数据聚合
```rust
impl<const N: usize> AllMeasurements<N> {
    pub fn to_usb_payload(self, otg_status: Option<OtgStatus>) -> AllMeasurementsUsbPayload {
        AllMeasurementsUsbPayload {
            // ... 现有字段 ...
            
            // OTG字段
            otg_enabled: otg_status.map(|s| s.enabled as u8).unwrap_or(0),
            otg_output_voltage_mv: otg_status.map(|s| s.output_voltage_mv).unwrap_or(0),
            otg_output_current_ma: otg_status.map(|s| s.output_current_ma).unwrap_or(0),
            otg_input_voltage_mv: otg_status.map(|s| s.input_voltage_mv).unwrap_or(0),
            otg_control_state: otg_status.map(|s| s.control_state as u8).unwrap_or(0),
        }
    }
}
```

### 5.2 USB任务扩展
```rust
// 在usb_task中订阅OTG状态
let mut otg_status_subscriber = otg_status_channel.subscriber().unwrap();
let mut latest_otg_status: Option<OtgStatus> = None;

// 在数据聚合循环中
if let Some(embassy_sync::pubsub::WaitResult::Message(otg_status)) =
    otg_status_subscriber.try_next_message()
{
    latest_otg_status = Some(otg_status);
}

// 生成USB数据包
let usb_payload = all_measurements.to_usb_payload(latest_otg_status);
```

## 6. 错误处理和调试

### 6.1 错误类型定义
```rust
#[derive(Debug)]
pub enum OtgError {
    I2cCommunication,
    DeviceInitialization,
    VoltageSetFailed,
    CurrentOverload,
    ConfigurationError,
}
```

### 6.2 调试信息
```rust
// 在OTG任务中添加详细日志
info!(
    "[OTG] Input: {}V, Output: {}mV, Current: {}mA, State: {:?}",
    input_voltage_v,
    current_output_voltage_mv,
    output_current_ma,
    control_state
);

// 每10秒输出一次状态摘要
static mut LAST_SUMMARY_TIME: u32 = 0;
let current_time = embassy_time::Instant::now().as_millis() as u32;
unsafe {
    if current_time - LAST_SUMMARY_TIME > 10000 {
        info!(
            "[OTG Summary] Enabled: {}, Vs: {}mV, Thresholds: {}%-{}%, Current limit: {}mA",
            config.enabled,
            config.target_voltage_mv,
            config.low_threshold_percent,
            config.high_threshold_percent,
            config.current_limit_ma
        );
        LAST_SUMMARY_TIME = current_time;
    }
}
```

这个详细设计文档提供了OTG功能的完整实现细节，包括数据结构、任务实现、系统集成和错误处理等所有方面。
