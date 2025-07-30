//! LED状态指示任务
//!
//! 管理GP25引脚上的LED，根据系统状态显示不同的闪烁模式：
//! - 存在任何故障时，LED 以 4Hz频率闪烁
//! - 充电时，0.5 Hz 频率闪烁
//! - 放电时，10100000 节奏闪烁（1亮0不亮，每位 0.25 秒）
//! - 充满时，111011110 节奏闪烁
//!
//! 按优先级从高到低执行，同一时间只显示一个匹配的情况。

use defmt::*;
use embassy_rp::gpio::Output;
use embassy_time::{Duration, Timer};

use crate::data_types::{Bq76920Alerts, Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    Bq76920AlertsSubscriber, Sc8815AlertsSubscriber, Sc8815MeasurementsSubscriber,
};

use bq769x0_async_rs::registers::SysStatFlags;

/// LED状态枚举
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
#[allow(dead_code)]
pub enum LedStatus {
    /// 系统初始化中 - 2Hz中速闪烁 (温和的初始化指示)
    Initializing,
    /// 故障状态 - 4Hz快速闪烁 (紧急故障警告)
    Fault,
    /// 充电状态 - 0.5Hz慢闪烁
    Charging,
    /// 放电状态 - 10100000节奏闪烁
    Discharging,
    /// 充满状态 - 111011110节奏闪烁
    ChargingComplete,
    /// 系统正常运行 - 1Hz心跳闪烁
    SystemActive,
    /// 正常状态 - LED关闭
    Normal,
}

/// LED状态指示任务
#[embassy_executor::task]
pub async fn led_status_task(
    led_pin: Output<'static>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    mut sc8815_measurements_subscriber: Sc8815MeasurementsSubscriber<'static>,
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,
) {
    info!("LED status task started");

    // 配置LED为推挽输出，高使能（RP2040板载LED）
    let mut led = led_pin;

    // 启动时测试LED - 快速闪烁3次确认LED工作
    info!("Testing LED functionality...");
    for _ in 0..3 {
        led.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led.set_low();
        Timer::after(Duration::from_millis(100)).await;
    }
    info!("LED test completed");

    led.set_low(); // 初始状态LED关闭（低电平）

    let mut current_status = LedStatus::Initializing; // 启动时强制进入初始化状态
    let mut pattern_index = 0;
    let mut last_update = embassy_time::Instant::now();

    // 系统状态跟踪
    let startup_time = embassy_time::Instant::now();
    let initialization_timeout = Duration::from_secs(30); // 30秒初始化超时

    // 设备状态跟踪
    let mut sc8815_initialized = false;
    let mut bq76920_initialized = false;
    let mut sc8815_last_seen = None::<embassy_time::Instant>;
    let mut bq76920_last_seen = None::<embassy_time::Instant>;

    // 保存最新的数据用于状态评估
    let mut latest_sc8815_alerts: Option<Sc8815Alerts> = None;
    let mut latest_sc8815_measurements: Option<Sc8815Measurements> = None;
    let mut latest_bq76920_alerts: Option<Bq76920Alerts> = None;

    loop {
        let now = embassy_time::Instant::now();

        // 检查SC8815数据更新（非阻塞）
        if let Some(embassy_sync::pubsub::WaitResult::Message(alerts)) =
            sc8815_alerts_subscriber.try_next_message()
        {
            latest_sc8815_alerts = Some(alerts);
            sc8815_last_seen = Some(now);
            if !sc8815_initialized {
                sc8815_initialized = true;
                info!("SC8815 device initialized and responding");
            }
        }

        if let Some(embassy_sync::pubsub::WaitResult::Message(measurements)) =
            sc8815_measurements_subscriber.try_next_message()
        {
            latest_sc8815_measurements = Some(measurements);
            sc8815_last_seen = Some(now);
            if !sc8815_initialized {
                sc8815_initialized = true;
                info!("SC8815 device initialized and responding");
            }
        }

        // 检查BQ76920数据更新（非阻塞）
        if let Some(embassy_sync::pubsub::WaitResult::Message(alerts)) =
            bq76920_alerts_subscriber.try_next_message()
        {
            latest_bq76920_alerts = Some(alerts);
            bq76920_last_seen = Some(now);
            if !bq76920_initialized {
                bq76920_initialized = true;
                info!("BQ76920 device initialized and responding");
            }
        }

        // 确定系统状态
        let new_status = determine_system_status(
            now,
            startup_time,
            initialization_timeout,
            sc8815_initialized,
            bq76920_initialized,
            sc8815_last_seen,
            bq76920_last_seen,
            &latest_sc8815_alerts,
            &latest_sc8815_measurements,
            &latest_bq76920_alerts,
        );

        // 每5秒输出一次状态调试信息
        static mut LAST_DEBUG_TIME: u32 = 0;
        let current_time = now.as_millis() as u32;
        unsafe {
            if current_time - LAST_DEBUG_TIME > 5000 {
                info!(
                    "LED Debug - Status: {:?}, SC8815_init: {}, BQ76920_init: {}",
                    new_status, sc8815_initialized, bq76920_initialized
                );
                LAST_DEBUG_TIME = current_time;
            }
        }

        // 如果状态改变，重置模式索引
        if new_status != current_status {
            current_status = new_status;
            pattern_index = 0;
            last_update = embassy_time::Instant::now();
            info!("LED status changed to: {:?}", current_status);
        }

        // 根据当前状态执行LED控制
        let now = embassy_time::Instant::now();
        match current_status {
            LedStatus::Initializing => {
                // 2Hz闪烁 (500ms周期，250ms亮，250ms灭) - 系统初始化中
                if now.duration_since(last_update) >= Duration::from_millis(250) {
                    led.toggle();
                    last_update = now;
                }
            }
            LedStatus::Fault => {
                // 4Hz快闪 (250ms周期，125ms亮，125ms灭) - 故障状态
                if now.duration_since(last_update) >= Duration::from_millis(125) {
                    led.toggle();
                    last_update = now;
                }
            }
            LedStatus::Charging => {
                // 0.5Hz闪烁 (2000ms周期，1000ms亮，1000ms灭)
                if now.duration_since(last_update) >= Duration::from_millis(1000) {
                    led.toggle();
                    last_update = now;
                }
            }
            LedStatus::SystemActive => {
                // 1Hz心跳闪烁 (1000ms周期，100ms亮，900ms灭)
                let cycle_time = now.duration_since(last_update);
                if cycle_time >= Duration::from_millis(1000) {
                    // 重新开始周期
                    led.set_high();
                    last_update = now;
                } else if cycle_time >= Duration::from_millis(100) {
                    // 100ms后关闭LED
                    led.set_low();
                }
            }
            LedStatus::Discharging => {
                // 10100000节奏闪烁，每位250ms
                execute_pattern(
                    &mut led,
                    &mut pattern_index,
                    &mut last_update,
                    &[true, false, true, false, false, false, false, false],
                    Duration::from_millis(250),
                );
            }
            LedStatus::ChargingComplete => {
                // 111011110节奏闪烁，每位250ms
                execute_pattern(
                    &mut led,
                    &mut pattern_index,
                    &mut last_update,
                    &[true, true, true, false, true, true, true, true, false],
                    Duration::from_millis(250),
                );
            }
            LedStatus::Normal => {
                // LED关闭
                led.set_low();
            }
        }

        // 短暂延时避免CPU占用过高
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// 执行特定的闪烁模式
fn execute_pattern(
    led: &mut Output<'static>,
    pattern_index: &mut usize,
    last_update: &mut embassy_time::Instant,
    pattern: &[bool],
    bit_duration: Duration,
) {
    let now = embassy_time::Instant::now();

    if now.duration_since(*last_update) >= bit_duration {
        if *pattern_index < pattern.len() {
            if pattern[*pattern_index] {
                led.set_high(); // LED亮（高使能，RP2040板载LED）
            } else {
                led.set_low(); // LED灭
            }
            *pattern_index += 1;
        } else {
            // 模式结束，重新开始
            *pattern_index = 0;
        }
        *last_update = now;
    }
}

/// 评估SC8815状态
fn evaluate_sc8815_status(alerts: &Sc8815Alerts, measurements: &Sc8815Measurements) -> LedStatus {
    let status = &alerts.device_status;
    let adc_measurements = &measurements.adc_measurements;

    // 检查故障状态（最高优先级）
    if status.otp_fault || status.vbus_short_fault {
        return LedStatus::Fault;
    }

    // 检查是否有充电器连接 (VBUS > 5V)
    if adc_measurements.vbus_mv > 5000 {
        // 有充电器连接，检查充电状态
        if status.eoc {
            // EOC=true 表示充电完成，没有在充电
            return LedStatus::ChargingComplete;
        } else {
            // EOC=false 表示正在充电（默认情况下充电功能是开启的）
            return LedStatus::Charging;
        }
    }

    // 没有充电器连接，检查是否在放电
    // TODO: 添加放电状态检测逻辑
    // 目前放电功能暂未实现，所以暂时不检测

    LedStatus::Normal
}

/// 确定系统整体状态
#[allow(clippy::too_many_arguments)]
fn determine_system_status(
    now: embassy_time::Instant,
    startup_time: embassy_time::Instant,
    initialization_timeout: Duration,
    sc8815_initialized: bool,
    bq76920_initialized: bool,
    sc8815_last_seen: Option<embassy_time::Instant>,
    bq76920_last_seen: Option<embassy_time::Instant>,
    sc8815_alerts: &Option<Sc8815Alerts>,
    sc8815_measurements: &Option<Sc8815Measurements>,
    bq76920_alerts: &Option<Bq76920Alerts>,
) -> LedStatus {
    let system_age = now.duration_since(startup_time);

    // 1. 检查是否在初始化阶段
    if system_age < initialization_timeout {
        if !sc8815_initialized && !bq76920_initialized {
            return LedStatus::Initializing;
        }
    } else {
        // 初始化超时，检查是否有设备未响应
        if !sc8815_initialized || !bq76920_initialized {
            return LedStatus::Fault;
        }
    }

    // 2. 检查设备通信超时（5秒无数据视为通信故障）
    let comm_timeout = Duration::from_secs(5);
    if let Some(last_seen) = sc8815_last_seen {
        if now.duration_since(last_seen) > comm_timeout {
            return LedStatus::Fault;
        }
    }
    if let Some(last_seen) = bq76920_last_seen {
        if now.duration_since(last_seen) > comm_timeout {
            return LedStatus::Fault;
        }
    }

    // 3. 检查BQ76920故障状态（最高优先级）
    if let Some(alerts) = bq76920_alerts {
        let fault_status = evaluate_bq76920_status(alerts);
        if matches!(fault_status, LedStatus::Fault) {
            return LedStatus::Fault;
        }
    }

    // 4. 检查SC8815状态并确定充电状态
    if let (Some(alerts), Some(measurements)) = (sc8815_alerts, sc8815_measurements) {
        let charging_status = evaluate_sc8815_status(alerts, measurements);
        // 如果不是充电状态，且系统正常运行，显示系统活跃状态
        if matches!(charging_status, LedStatus::Normal) && sc8815_initialized && bq76920_initialized
        {
            return LedStatus::SystemActive;
        }
        return charging_status;
    }

    // 5. 如果所有设备都已初始化但没有完整数据，显示系统活跃状态
    if sc8815_initialized && bq76920_initialized {
        return LedStatus::SystemActive;
    }

    // 6. 默认状态 - 只有在设备未初始化时才显示Normal
    LedStatus::Normal
}

/// 评估BQ76920状态
fn evaluate_bq76920_status(alerts: &Bq76920Alerts) -> LedStatus {
    let sys_stat = alerts.system_status.0;

    // 检查各种故障状态（最高优先级）
    if sys_stat.contains(SysStatFlags::OV) ||      // 过压
       sys_stat.contains(SysStatFlags::UV) ||      // 欠压
       sys_stat.contains(SysStatFlags::SCD) ||     // 短路放电
       sys_stat.contains(SysStatFlags::OCD)
    {
        // 过流放电
        return LedStatus::Fault;
    }

    LedStatus::Normal
}
