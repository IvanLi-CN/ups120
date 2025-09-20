//! LED状态指示任务（单灯规范）
//!
//! 优先级：Fault > Charging(+Bal overlay) > Full(hysteresis) > Idle。
//!
//! - Fault：4 Hz 闪烁（125ms 亮 / 125ms 灭）。
//! - Charging（基线）：1 Hz 闪烁（500ms 亮 / 500ms 灭）。
//! - Balancing 叠加：在 Charging 的亮窗内插入两个“40ms 灭”缺口，间隔 160ms；不改变基线节拍。
//! - Full：依据 VBAT 与 IBAT 的迟滞规则判定；显示为常亮。
//! - Idle：熄灭。

use defmt::*;
use embassy_stm32::gpio::OutputOpenDrain;
use embassy_time::{Duration, Timer};

use crate::data_types::{BalancingCvRequest, Bq76920Alerts, Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    BalancingCvRequestSubscriber, Bq76920AlertsSubscriber, Sc8815AlertsSubscriber,
    Sc8815MeasurementsSubscriber,
};

use bq769x0_async_rs::registers::SysStatFlags;

#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
enum LedMode {
    Fault,
    Charging,
    Full,
    Idle,
}

// 常量参数（若需按机型调整，可迁移到配置模块）
const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
// 终止电流阈值（LED判定用）。如需精调，可据实测调整。
const ITERM_MA: u16 = 100;
const ITERM_EXIT_MULTIPLIER_X10: u16 = 12; // 1.2x

// 进入满电的迟滞时间（45–90 s取中位）与退出时间
const FULL_ENTER_SECS: u32 = 60;
const FULL_EXIT_SECS: u32 = 10;

// 充电基线节拍与均衡叠加缺口定义
const CHG_PERIOD_MS: u32 = 1000;
const CHG_ON_MS: u32 = 500;
const NOTCH_WIDTH_MS: u32 = 40;
const NOTCH1_START_MS: u32 = 100;
const NOTCH_GAP_MS: u32 = 160; // 缺口间距

/// LED状态指示任务
#[embassy_executor::task]
pub async fn led_status_task(
    led_pin: OutputOpenDrain<'static>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    mut sc8815_measurements_subscriber: Sc8815MeasurementsSubscriber<'static>,
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,
    mut balancing_cv_subscriber: BalancingCvRequestSubscriber<'static>,
) {
    info!("LED status task started");

    // 配置LED为开漏输出，低使能
    let mut led = led_pin;
    led.set_high(); // 初始状态LED关闭（高电平）

    let mut mode = LedMode::Idle;
    let mut last_toggle = embassy_time::Instant::now(); // 给 Fault 用
    let mut chg_cycle_start = embassy_time::Instant::now();
    let mut full_enter_acc_ms: u32 = 0;
    let mut full_exit_acc_ms: u32 = 0;
    let mut is_full_latched = false;
    let mut overlay_prev: Option<bool> = None;

    // 保存最新的SC8815数据
    let mut latest_sc8815_alerts: Option<Sc8815Alerts> = None;
    let mut latest_sc8815_measurements: Option<Sc8815Measurements> = None;

    // 最新状态缓存
    let mut latest_balancing: BalancingCvRequest = BalancingCvRequest::default();

    loop {
        // 非阻塞抓取消息
        if let Some(msg) = balancing_cv_subscriber.try_next_message_pure() {
            latest_balancing = msg;
        }

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

        // 先检查 BQ 故障（最高优先级，立即生效）
        let mut any_fault = false;
        if let Some(bq76920_result) = bq76920_alerts_subscriber.try_next_message() {
            match bq76920_result {
                embassy_sync::pubsub::WaitResult::Message(alerts) => {
                    if evaluate_bq_fault(&alerts) {
                        any_fault = true;
                    }
                }
                embassy_sync::pubsub::WaitResult::Lagged(_) => {
                    // 忽略滞后消息，继续处理
                }
            }
        }

        let now = embassy_time::Instant::now();

        if any_fault {
            // 故障：4Hz 闪烁，抢占显示
            if now.duration_since(last_toggle) >= Duration::from_millis(125) {
                led.toggle();
                last_toggle = now;
            }
            // 处于故障时不更新其它状态的迟滞累计
            Timer::after(Duration::from_millis(10)).await;
            continue;
        }

        // 评估“充电/满电/空闲”与迟滞
        let (mut is_charging_policy, mut ac_present, mut ibat_ma, mut vbat_mv) =
            (false, false, 0u16, 0u16);
        if let (Some(alerts), Some(meas)) = (&latest_sc8815_alerts, &latest_sc8815_measurements) {
            is_charging_policy = alerts.expected_charging || alerts.charging_confirmed;
            ac_present = alerts.device_status.ac_adapter_connected;
            ibat_ma = meas.adc_measurements.ibat_ma;
            vbat_mv = meas.adc_measurements.vbat_mv;
        }

        // 维护满电迟滞锁存（只在适配器存在且无故障时考虑）
        if ac_present {
            let enter_ok = (vbat_mv as i32) >= PACK_CHARGE_STOP_THRESHOLD_MV && ibat_ma <= ITERM_MA;
            let exit_by_current =
                ibat_ma >= ((ITERM_MA as u32 * ITERM_EXIT_MULTIPLIER_X10 as u32 + 9) / 10) as u16; // >=1.2x
            let exit_by_voltage = (vbat_mv as i32) < PACK_CHARGE_START_THRESHOLD_MV; // 宽松“离开浮充带”

            if !is_full_latched {
                if enter_ok {
                    full_enter_acc_ms = (full_enter_acc_ms + 10).min((FULL_ENTER_SECS + 1) * 1000);
                } else {
                    full_enter_acc_ms = 0;
                }
                if full_enter_acc_ms >= FULL_ENTER_SECS * 1000 {
                    is_full_latched = true;
                    full_exit_acc_ms = 0;
                    info!("led_full_latched");
                }
            } else {
                // 满电已锁存，监测退出条件累计
                if exit_by_current || exit_by_voltage || !is_charging_policy {
                    full_exit_acc_ms = (full_exit_acc_ms + 10).min((FULL_EXIT_SECS + 1) * 1000);
                } else {
                    full_exit_acc_ms = 0;
                }
                if full_exit_acc_ms >= FULL_EXIT_SECS * 1000 {
                    is_full_latched = false;
                    full_enter_acc_ms = 0;
                    info!("led_full_released");
                }
            }
        } else {
            // 适配器不在时，清空满电状态
            is_full_latched = false;
            full_enter_acc_ms = 0;
            full_exit_acc_ms = 0;
        }

        // 模式选择（不含故障）：Charging 优先于 Full，再到 Idle
        let desired_mode = if is_charging_policy {
            LedMode::Charging
        } else if is_full_latched {
            LedMode::Full
        } else {
            LedMode::Idle
        };
        if desired_mode != mode {
            mode = desired_mode;
            if matches!(mode, LedMode::Charging) {
                chg_cycle_start = now;
            }
        }

        // 输出控制
        match mode {
            LedMode::Charging => {
                let elapsed_ms = now.duration_since(chg_cycle_start).as_millis() as u32;
                let phase = elapsed_ms % CHG_PERIOD_MS;
                let in_on = phase < CHG_ON_MS;
                let mut on = in_on;
                if in_on && latest_balancing.overlay {
                    // 插入两个 40ms 灭缺口： [100,140) 与 [300,340)
                    let notch2_start = NOTCH1_START_MS + NOTCH_GAP_MS + NOTCH_WIDTH_MS; // 100 + 160 + 40 = 300
                    let in_notch1 =
                        phase >= NOTCH1_START_MS && phase < (NOTCH1_START_MS + NOTCH_WIDTH_MS);
                    let in_notch2 =
                        phase >= notch2_start && phase < (notch2_start + NOTCH_WIDTH_MS);
                    if in_notch1 || in_notch2 {
                        on = false;
                    }
                }
                // 仅在叠加状态改变时记录一条日志，避免刷屏
                if overlay_prev
                    .map(|v| v != latest_balancing.overlay)
                    .unwrap_or(true)
                {
                    info!("led_chg overlay={}", latest_balancing.overlay);
                    overlay_prev = Some(latest_balancing.overlay);
                }
                if on {
                    led.set_low();
                } else {
                    led.set_high();
                }
            }
            LedMode::Full => {
                // 常亮（开漏低）
                led.set_low();
            }
            LedMode::Idle => {
                // 熄灭
                led.set_high();
            }
            LedMode::Fault => ::core::unreachable!(),
        }

        // 短暂延时避免CPU占用过高
        Timer::after(Duration::from_millis(10)).await;
    }
}

fn evaluate_bq_fault(alerts: &Bq76920Alerts) -> bool {
    let sys_stat = alerts.system_status.0;
    sys_stat.contains(SysStatFlags::OV)
        || sys_stat.contains(SysStatFlags::UV)
        || sys_stat.contains(SysStatFlags::SCD)
        || sys_stat.contains(SysStatFlags::OCD)
}
