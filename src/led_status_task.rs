//! LED Status Indication Task
//!
//! Manages the LED on GPIO25 pin, displaying different blink patterns based on system state:
//! - Fault: 4Hz fast blink when any component fault occurs
//! - Charging with Mains: Double flash every 3 seconds (mains power normal, battery charging)
//! - Mains without Charging: Triple flash every 3 seconds (mains power normal, battery full)
//! - Backup Power Output: Custom pattern (mains interrupted, battery discharging)
//! - Initialization: 2Hz medium blink during system startup
//!
//! Executes by priority from high to low, displaying only one matching condition at a time.

use defmt::*;
use embassy_rp::gpio::Output;
use embassy_time::{Duration, Timer};

use crate::data_types::{Bq76920Alerts, OtgStatus, Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    Bq76920AlertsSubscriber, OtgStatusSubscriber, Sc8815AlertsSubscriber,
    Sc8815MeasurementsSubscriber,
};

use bq769x0_async_rs::registers::SysStatFlags;

/// LED Status Enumeration
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
#[allow(dead_code)]
pub enum LedStatus {
    /// System initialization - 2Hz medium blink (gentle initialization indicator)
    Initialization,
    /// Fault state - 4Hz fast blink (any component fault or communication error)
    Fault,
    /// Charging with mains power - Double flash every 3 seconds
    ChargingWithMains,
    /// Mains power without charging - Triple flash every 3 seconds
    MainsWithoutCharging,
    /// Backup power output - Custom pattern (mains interrupted, battery discharging)
    BackupPowerOutput,
}

/// LED状态指示任务
#[embassy_executor::task]
pub async fn led_status_task(
    led_pin: Output<'static>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    mut sc8815_measurements_subscriber: Sc8815MeasurementsSubscriber<'static>,
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,
    mut otg_status_subscriber: OtgStatusSubscriber<'static>,
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

    let mut current_status = LedStatus::Initialization; // 启动时强制进入初始化状态
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
    let mut latest_otg_status: Option<OtgStatus> = None;

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

        // 检查OTG状态更新（非阻塞）
        if let Some(embassy_sync::pubsub::WaitResult::Message(otg_status)) =
            otg_status_subscriber.try_next_message()
        {
            latest_otg_status = Some(otg_status);
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
            &latest_otg_status,
            current_status,
        );

        // 每秒输出一次状态调试信息
        static mut LAST_DEBUG_TIME: u32 = 0;
        let current_time = now.as_millis() as u32;
        unsafe {
            if current_time - LAST_DEBUG_TIME > 1000 {
                info!(
                    "LED Debug - Status: {:?}, SC8815_init: {}, BQ76920_init: {}",
                    new_status, sc8815_initialized, bq76920_initialized
                );
                info!("LED当前状态: {:?}", new_status);
                LAST_DEBUG_TIME = current_time;
            }
        }

        // 如果状态改变，重置模式索引
        if new_status != current_status {
            info!(
                "LED status changed: {:?} -> {:?}",
                current_status, new_status
            );
            current_status = new_status;
            // Don't reset last_update to avoid disrupting LED patterns
        }

        // Execute LED control based on current status
        let now = embassy_time::Instant::now();
        match current_status {
            LedStatus::Initialization => {
                // 2Hz blink: 250ms on, 250ms off - System initialization
                if now.duration_since(last_update) >= Duration::from_millis(250) {
                    led.toggle();
                    last_update = now;
                }
            }
            LedStatus::Fault => {
                // 4Hz fast blink: 125ms on, 125ms off - Fault state
                if now.duration_since(last_update) >= Duration::from_millis(125) {
                    led.toggle();
                    last_update = now;
                }
            }
            LedStatus::ChargingWithMains => {
                // Double flash every 3 seconds - use absolute time to avoid reset issues
                let cycle_time = now.as_millis() % 3000;
                match cycle_time {
                    0..100 => led.set_high(), // First flash
                    100..200 => led.set_low(),
                    200..300 => led.set_high(), // Second flash
                    300..3000 => led.set_low(), // Long off period
                    _ => {}
                }
            }
            LedStatus::MainsWithoutCharging => {
                // Triple flash every 3 seconds - use absolute time to avoid reset issues
                let cycle_time = now.as_millis() % 3000;
                match cycle_time {
                    0..100 => led.set_high(), // First flash
                    100..200 => led.set_low(),
                    200..300 => led.set_high(), // Second flash
                    300..400 => led.set_low(),
                    400..500 => led.set_high(), // Third flash
                    500..3000 => led.set_low(), // Long off period
                    _ => {}
                }
            }
            LedStatus::BackupPowerOutput => {
                // Custom pattern for backup power mode - 1Hz heartbeat for now
                let cycle_time = now.duration_since(last_update);
                if cycle_time >= Duration::from_millis(1000) {
                    led.set_high();
                    last_update = now;
                } else if cycle_time >= Duration::from_millis(100) {
                    led.set_low();
                }
            }
        }

        // 短暂延时避免CPU占用过高
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Evaluate SC8815 status for charging states
fn evaluate_sc8815_status(alerts: &Sc8815Alerts, measurements: &Sc8815Measurements) -> LedStatus {
    let status = &alerts.device_status;
    let adc_measurements = &measurements.adc_measurements;

    // Check fault conditions (highest priority)
    if status.otp_fault || status.vbus_short_fault {
        return LedStatus::Fault;
    }

    // Check if charger is connected (VBUS > 5V) - indicates mains power available
    if adc_measurements.vbus_mv > 5000 {
        // Mains power available, check charging status
        if status.eoc {
            // EOC=true means charging complete, not actively charging
            return LedStatus::MainsWithoutCharging;
        } else {
            // EOC=false means actively charging
            return LedStatus::ChargingWithMains;
        }
    }

    // No charger connected - this could indicate backup power mode
    // Return a neutral state, let the main logic decide
    LedStatus::MainsWithoutCharging
}

/// Determine overall system status based on priority
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
    otg_status: &Option<OtgStatus>,
    current_status: LedStatus,
) -> LedStatus {
    let system_age = now.duration_since(startup_time);

    // Priority 1: Check if in initialization phase
    if system_age < initialization_timeout {
        if !sc8815_initialized && !bq76920_initialized {
            return LedStatus::Initialization;
        }
    } else {
        // Initialization timeout, check if any device is unresponsive
        if !sc8815_initialized || !bq76920_initialized {
            return LedStatus::Fault;
        }
    }

    // Priority 2: Check device communication timeout (5 seconds without data = fault)
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

    // Priority 3: Check BQ76920 fault conditions (highest priority for faults)
    if let Some(alerts) = bq76920_alerts {
        let fault_status = evaluate_bq76920_status(alerts);
        if matches!(fault_status, LedStatus::Fault) {
            return LedStatus::Fault;
        }
    }

    // Priority 4: Check OTG fault conditions
    if has_otg_fault(otg_status, now) {
        return LedStatus::Fault;
    }

    // Priority 5: Check backup power output (OTG discharging) with hysteresis
    if let Some(otg) = otg_status {
        if otg.enabled {
            // Use hysteresis to prevent rapid switching
            // If currently in backup mode, need current to drop below 80mA to switch out
            // If not in backup mode, need current to exceed 120mA to switch in
            let threshold = match current_status {
                LedStatus::BackupPowerOutput => 80, // Lower threshold to exit backup mode
                _ => 120,                           // Higher threshold to enter backup mode
            };

            if otg.output_current_ma > threshold {
                return LedStatus::BackupPowerOutput;
            }
        }
    }

    // Priority 6: Check SC8815 status for mains power and charging states
    if let (Some(alerts), Some(measurements)) = (sc8815_alerts, sc8815_measurements) {
        let charging_status = evaluate_sc8815_status(alerts, measurements);
        return charging_status;
    }

    // Priority 7: If all devices initialized but no complete data, default to mains without charging
    if sc8815_initialized && bq76920_initialized {
        return LedStatus::MainsWithoutCharging;
    }

    // Fallback: Initialization state if devices not ready
    LedStatus::Initialization
}

/// 检查OTG是否有故障
fn has_otg_fault(otg_status: &Option<OtgStatus>, now: embassy_time::Instant) -> bool {
    if let Some(otg) = otg_status {
        // 检查通信超时（5秒）
        let last_update_instant = embassy_time::Instant::from_millis(otg.last_update_ms);
        let time_since_update = now.duration_since(last_update_instant);
        if time_since_update > Duration::from_secs(5) {
            return true;
        }

        // 检查过载保护（1.2A = 1A + 20%容差）
        if otg.output_current_ma > 1200 {
            return true;
        }

        // 检查OTG启用但输出电压为0（可能的故障）
        if otg.enabled && otg.output_voltage_mv == 0 {
            return true;
        }
    }

    false
}

/// Evaluate BQ76920 status for fault conditions
fn evaluate_bq76920_status(alerts: &Bq76920Alerts) -> LedStatus {
    let sys_stat = alerts.system_status.0;

    // Check various fault conditions (highest priority)
    if sys_stat.contains(SysStatFlags::OV) ||      // Overvoltage
       sys_stat.contains(SysStatFlags::UV) ||      // Undervoltage
       sys_stat.contains(SysStatFlags::SCD) ||     // Short circuit discharge
       sys_stat.contains(SysStatFlags::OCD)
    // Overcurrent discharge
    {
        return LedStatus::Fault;
    }

    // No faults detected, return a neutral state
    LedStatus::MainsWithoutCharging
}
