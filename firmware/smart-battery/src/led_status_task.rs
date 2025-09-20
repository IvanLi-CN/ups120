//! LED状态指示任务
//!
//! 管理PA5引脚上的LED，根据系统状态显示不同的闪烁模式：
//! - 存在任何故障时，LED 以 4Hz频率闪烁
//! - 充电时，0.5 Hz 频率闪烁
//! - 放电时，10100000 节奏闪烁（1亮0不亮，每位 0.25 秒）
//! - 充满时，111011110 节奏闪烁
//!
//! 按优先级从高到低执行，同一时间只显示一个匹配的情况。

use defmt::*;
use embassy_stm32::gpio::OutputOpenDrain;
use embassy_time::{Duration, Timer};

use crate::data_types::{Bq76920Alerts, Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    Bq76920AlertsSubscriber, Sc8815AlertsSubscriber, Sc8815MeasurementsSubscriber,
};

use bq769x0_async_rs::registers::SysStatFlags;

/// LED状态枚举
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum LedStatus {
    /// 故障状态 - 4Hz闪烁
    Fault,
    /// 充电状态 - 0.5Hz闪烁
    Charging,
    /// 放电状态 - 10100000节奏闪烁
    Discharging,
    /// 充满状态 - 111011110节奏闪烁
    ChargingComplete,
    /// 正常状态 - LED关闭
    Normal,
}

/// LED状态指示任务
#[embassy_executor::task]
pub async fn led_status_task(
    led_pin: OutputOpenDrain<'static>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    mut sc8815_measurements_subscriber: Sc8815MeasurementsSubscriber<'static>,
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,
) {
    info!("LED status task started");

    // 配置LED为开漏输出，低使能
    let mut led = led_pin;
    led.set_high(); // 初始状态LED关闭（高电平）

    let mut current_status = LedStatus::Normal;
    let mut pattern_index = 0;
    let mut last_update = embassy_time::Instant::now();

    // 保存最新的SC8815数据
    let mut latest_sc8815_alerts: Option<Sc8815Alerts> = None;
    let mut latest_sc8815_measurements: Option<Sc8815Measurements> = None;

    loop {
        // 检查是否有新的状态更新
        let mut new_status = LedStatus::Normal;

        // 检查SC8815告警状态（非阻塞）
        if let Some(sc8815_result) = sc8815_alerts_subscriber.try_next_message() {
            match sc8815_result {
                embassy_sync::pubsub::WaitResult::Message(alerts) => {
                    latest_sc8815_alerts = Some(alerts);
                }
                embassy_sync::pubsub::WaitResult::Lagged(_) => {
                    // 忽略滞后消息，继续处理
                }
            }
        }

        // 检查SC8815测量数据（非阻塞）
        if let Some(sc8815_measurements_result) = sc8815_measurements_subscriber.try_next_message()
        {
            match sc8815_measurements_result {
                embassy_sync::pubsub::WaitResult::Message(measurements) => {
                    latest_sc8815_measurements = Some(measurements);
                }
                embassy_sync::pubsub::WaitResult::Lagged(_) => {
                    // 忽略滞后消息，继续处理
                }
            }
        }

        // 如果有SC8815数据，评估状态
        if let (Some(alerts), Some(measurements)) =
            (&latest_sc8815_alerts, &latest_sc8815_measurements)
        {
            new_status = evaluate_sc8815_status(alerts, measurements);
        }

        // 检查BQ76920告警状态（非阻塞）
        if let Some(bq76920_result) = bq76920_alerts_subscriber.try_next_message() {
            match bq76920_result {
                embassy_sync::pubsub::WaitResult::Message(alerts) => {
                    let bq_status = evaluate_bq76920_status(&alerts);
                    // BQ76920故障优先级更高
                    if matches!(bq_status, LedStatus::Fault) {
                        new_status = bq_status;
                    }
                }
                embassy_sync::pubsub::WaitResult::Lagged(_) => {
                    // 忽略滞后消息，继续处理
                }
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
            LedStatus::Fault => {
                // 4Hz闪烁 (250ms周期，125ms亮，125ms灭)
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
                led.set_high();
            }
        }

        // 短暂延时避免CPU占用过高
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// 执行特定的闪烁模式
fn execute_pattern(
    led: &mut OutputOpenDrain<'static>,
    pattern_index: &mut usize,
    last_update: &mut embassy_time::Instant,
    pattern: &[bool],
    bit_duration: Duration,
) {
    let now = embassy_time::Instant::now();

    if now.duration_since(*last_update) >= bit_duration {
        if *pattern_index < pattern.len() {
            if pattern[*pattern_index] {
                led.set_low(); // LED亮（低使能）
            } else {
                led.set_high(); // LED灭
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

    // 检查充电完成状态
    if status.eoc && status.ac_adapter_connected {
        return LedStatus::ChargingComplete;
    }

    // 根据确认标志判断是否正在充电
    if alerts.charging_confirmed {
        return LedStatus::Charging;
    }

    // 若策略希望充电但尚未检测到有效电流，保持待机显示（LED灭）
    if alerts.expected_charging && status.ac_adapter_connected {
        return LedStatus::Normal;
    }

    // 没有确认充电，再根据适配器检测或电压判断普通连接状态
    if status.ac_adapter_connected || adc_measurements.vbus_mv > 5000 {
        return LedStatus::Normal;
    }

    // 没有充电器连接，检查是否在放电
    // TODO: 添加放电状态检测逻辑
    // 目前放电功能暂未实现，所以暂时不检测

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
