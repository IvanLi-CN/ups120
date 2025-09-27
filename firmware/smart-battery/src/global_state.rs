//! Battery Global State aggregation and publishing.
//!
//! This module derives a concise global state from low-level alerts/measurements
//! and exposes it via a PubSub channel. Other tasks (LED, logging, etc.)
//! can subscribe to react immediately to changes.

use defmt::*;
use embassy_time::{Duration, Instant, Timer};

use crate::data_types::{
    BalancingCvRequest, Bq76920Alerts, Bq76920Measurements, Sc8815Alerts, Sc8815Measurements,
};
use crate::shared::{
    BalancingCvRequestSubscriber, Bq76920AlertsSubscriber, Bq76920MeasurementsSubscriber,
    GlobalStatePublisher, Sc8815AlertsSubscriber, Sc8815MeasurementsSubscriber,
};
use bq769x0_async_rs::registers::SysStatFlags;

// Thresholds and hysteresis used to determine "full" status
const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
const ITERM_MA: u16 = 100; // termination current for "full" determination
const ITERM_EXIT_MULTIPLIER_X10: u16 = 12; // 1.2x exit from full latch
const FULL_ENTER_SECS: u32 = 60;
const FULL_EXIT_SECS: u32 = 10;

/// Derived, compact system state for UI/LED/logging.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BatteryGlobalState {
    pub ac_present: bool,
    pub charging: bool,         // include paused-as-charging phase
    pub charging_paused: bool,  // OV/imbalance pause gating the power stage
    pub preparing: bool, // desire/need to charge but AC not present (or session not yet ready)
    pub full: bool,      // hysteresis-latched “full”
    pub balancing_active: bool, // HW balancing active (overlay indicator)
    pub fault_battery: bool, // BQ OV/UV/SCD/OCD
    pub fault_charger: bool, // SC8815 OTP/VBUS short
}

impl Default for BatteryGlobalState {
    fn default() -> Self {
        Self {
            ac_present: false,
            charging: false,
            charging_paused: false,
            preparing: false,
            full: false,
            balancing_active: false,
            fault_battery: false,
            fault_charger: false,
        }
    }
}

fn eval_bq_fault(alerts: &Bq76920Alerts) -> bool {
    let f = alerts.system_status.0;
    f.contains(SysStatFlags::OV)
        || f.contains(SysStatFlags::UV)
        || f.contains(SysStatFlags::SCD)
        || f.contains(SysStatFlags::OCD)
}

/// Aggregation task that publishes `BatteryGlobalState` whenever it changes.
#[embassy_executor::task]
pub async fn global_state_task(
    mut sc_alerts_sub: Sc8815AlertsSubscriber<'static>,
    mut sc_meas_sub: Sc8815MeasurementsSubscriber<'static>,
    mut bq_alerts_sub: Bq76920AlertsSubscriber<'static>,
    mut bal_cv_sub: BalancingCvRequestSubscriber<'static>,
    mut bq_meas_sub: Bq76920MeasurementsSubscriber<'static, 5>,
    state_pub: GlobalStatePublisher<'static>,
) {
    debug!("gs:start");

    // Latest inputs (non-blocking sampling)
    let mut latest_sc_alerts: Option<Sc8815Alerts> = None;
    let mut latest_sc_meas: Option<Sc8815Measurements> = None;
    let mut latest_bq_alerts: Option<Bq76920Alerts> = None;
    let mut latest_bq_meas: Option<Bq76920Measurements<5>> = None;
    let mut latest_bal: BalancingCvRequest = BalancingCvRequest::default();

    // Full-state hysteresis latches
    let mut full_enter_acc_ms: u32 = 0;
    let mut full_exit_acc_ms: u32 = 0;
    let mut is_full_latched = false;

    let mut last_published: BatteryGlobalState = BatteryGlobalState::default();
    let mut first_pub = true;
    let last_eval = Instant::now();

    loop {
        if let Some(a) = sc_alerts_sub.try_next_message_pure() {
            latest_sc_alerts = Some(a);
        }
        if let Some(m) = sc_meas_sub.try_next_message_pure() {
            latest_sc_meas = Some(m);
        }
        if let Some(a) = bq_alerts_sub.try_next_message_pure() {
            latest_bq_alerts = Some(a);
        }
        if let Some(m) = bq_meas_sub.try_next_message_pure() {
            latest_bq_meas = Some(m);
        }
        if let Some(b) = bal_cv_sub.try_next_message_pure() {
            latest_bal = b;
        }

        // Derive booleans
        let mut ac_present = false;
        let mut charger_fault = false;
        let mut charging_flags = false;
        let mut charging_paused = false;
        let mut preparing = false;

        if let Some(a) = latest_sc_alerts.as_ref() {
            ac_present = a.device_status.ac_adapter_connected;
            charger_fault = a.device_status.otp_fault || a.device_status.vbus_short_fault;
            charging_paused = (a.ov_pause_active || a.imbalance_pause_active) && ac_present;
            let charging_active = (a.expected_charging || a.charging_confirmed) && ac_present;
            charging_flags = charging_active || charging_paused;
        }

        let mut fault_battery = false;
        if let Some(bqa) = latest_bq_alerts.as_ref() {
            fault_battery = eval_bq_fault(bqa);
        }

        // Full determination with hysteresis (requires SC measurements)
        if ac_present {
            if let Some(meas) = latest_sc_meas.as_ref() {
                let vbat_mv = meas.adc_measurements.vbat_mv as i32;
                let ibat_ma = meas.adc_measurements.ibat_ma;
                let enter_ok = vbat_mv >= PACK_CHARGE_STOP_THRESHOLD_MV && ibat_ma <= ITERM_MA;
                let exit_by_current = ibat_ma
                    >= ((ITERM_MA as u32 * ITERM_EXIT_MULTIPLIER_X10 as u32 + 9) / 10) as u16;
                let exit_by_voltage = vbat_mv < PACK_CHARGE_START_THRESHOLD_MV;

                // 满电判据：SC EOC 或 BQ 迟滞（二者任一即可）
                let sc_eoc = latest_sc_alerts
                    .as_ref()
                    .map(|a| a.device_status.eoc)
                    .unwrap_or(false);
                if sc_eoc {
                    is_full_latched = true;
                    full_exit_acc_ms = 0;
                }

                if !is_full_latched {
                    if enter_ok {
                        full_enter_acc_ms =
                            (full_enter_acc_ms + 10).min((FULL_ENTER_SECS + 1) * 1000);
                    } else {
                        full_enter_acc_ms = 0;
                    }
                    if full_enter_acc_ms >= FULL_ENTER_SECS * 1000 {
                        is_full_latched = true;
                        full_exit_acc_ms = 0;
                        debug!("full=1");
                    }
                } else {
                    if exit_by_current || exit_by_voltage || !charging_flags {
                        full_exit_acc_ms = (full_exit_acc_ms + 10).min((FULL_EXIT_SECS + 1) * 1000);
                    } else {
                        full_exit_acc_ms = 0;
                    }
                    if full_exit_acc_ms >= FULL_EXIT_SECS * 1000 {
                        is_full_latched = false;
                        full_enter_acc_ms = 0;
                        debug!("full=0");
                    }
                }
            }
        } else {
            // No adapter → clear full latch
            is_full_latched = false;
            full_enter_acc_ms = 0;
            full_exit_acc_ms = 0;
        }

        // Preparing-to-charge: 无 AC 时若电压低于启动阈值则提示。
        // 优先使用 BQ 测量（低功耗、始终在线），若无则退回 SC 测量。
        if !ac_present {
            let mut vbat_mv_opt: Option<i32> = None;
            if let Some(bq) = latest_bq_meas.as_ref() {
                vbat_mv_opt = Some(bq.core_measurements.total_voltage_mv);
            } else if let Some(sc) = latest_sc_meas.as_ref() {
                vbat_mv_opt = Some(sc.adc_measurements.vbat_mv as i32);
            }
            if let Some(vbat_mv) = vbat_mv_opt {
                preparing = vbat_mv < PACK_CHARGE_START_THRESHOLD_MV && !is_full_latched;
            }
        }

        let new_state = BatteryGlobalState {
            ac_present,
            charging: charging_flags,
            charging_paused,
            preparing,
            full: is_full_latched,
            balancing_active: latest_bal.overlay,
            fault_battery,
            fault_charger: charger_fault,
        };

        if first_pub || new_state != last_published {
            // snapshot omitted to save flash; state is visible via I2C + event logs
            state_pub.publish_immediate(new_state);
            last_published = new_state;
            first_pub = false;
        }

        // 当 AC 缺失时，保持低频评估而不是完全阻塞，
        // 以便能够吸收来自 SC8815 的“适配器插入”提示并发布新全局状态。
        if crate::scheduler::is_quiesced() {
            Timer::after(Duration::from_millis(250)).await;
            continue;
        }

        // AC present: evaluate every 10 ms for responsive UI/LED and latching logic.
        let now = Instant::now();
        let _ = now;
        let _ = last_eval; // reserved if later needed
        Timer::after(Duration::from_millis(10)).await;
    }
}
