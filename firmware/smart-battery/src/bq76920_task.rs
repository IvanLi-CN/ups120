use bq769x0_async_rs::RegisterAccess;
use bq769x0_async_rs::registers::{Register, SysCtrl2Flags, SysStatFlags};
use defmt::*;
use embassy_time::{Duration, Timer, with_timeout};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
// EXTI is handled by irq_mux; no direct dependency here
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use portable_atomic::AtomicBool;

use bq769x0_async_rs::ProtectionConfig;
use bq769x0_async_rs::{
    BatteryConfig, Bq769x0, data_types::NtcParameters, errors::Error as BQ769x0Error,
};

// Import necessary data types
use crate::charger_control;
use crate::data_types::BalancingCvRequest;
use crate::i2c_slave;
use crate::shared::{
    BalancingCvRequestPublisher, Bq76920AlertsPublisher, Bq76920MeasurementsPublisher,
    Sc8815AlertsSubscriber,
};
use crate::state_bits::{self, bits as sbits};
use crate::temp_policy::{self, TempPolicyOutput, TempPolicyState};
use crate::thermal::{self, TEMP_INVALID_0_01C};

const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
const PACK_OUTPUT_CUTOFF_THRESHOLD_MV: i32 = 12_500;
// New balancing policy thresholds
// Start balancing during charging when pack spread (max-min) exceeds 10 mV.
const BALANCE_START_SPREAD_MV: i32 = 10;
const LOG_CELL_DELTA: bool = true; // log cell voltages/min/max/spread each sample for diagnostics
// A local peak must exceed at least one adjacent cell by >1 mV to be eligible.
const LOCAL_PEAK_MARGIN_MV: i32 = 1;

// Logging verbosity toggles for BQ76920 task
const VERBOSE_BQ_LOG: bool = false; // set true for full register-by-register dumps
// snapshot disabled to save flash

// Test knob: force both CHG/DSG FETs off for charger-path diagnostics.
// Default false for normal operation; set true only for lab diagnostics.
const TEST_FORCE_BQ_FETS_OFF: bool = false;
// Interlock deadtime when switching balancing target cells (safety)
const BALANCE_SWITCH_DEADTIME_MS: u64 = 40;
// EMA coefficient in percent (for active mode temperature smoothing)
const TEMP_EMA_ALPHA_PCT: i32 = 20; // 20% new sample, 80% history

// Wake-on-ALERT pending flag for BQ (set by EXTI task)
static BQ_ALERT_PENDING: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn set_bq_alert_pending() {
    BQ_ALERT_PENDING.store(true, portable_atomic::Ordering::Relaxed);
    crate::sleep_manager::bump("bq-int");
}

#[inline(always)]
fn update_bq_state(preparing: bool, balancing_active: bool, fault_bq: bool, active: bool) {
    const MASK: u16 = sbits::PREPARING | sbits::BALANCING | sbits::FAULT_BQ | sbits::ACTIVE_BQ;
    let mut value = 0u16;
    if preparing {
        value |= sbits::PREPARING;
    }
    if balancing_active {
        value |= sbits::BALANCING;
    }
    if fault_bq {
        value |= sbits::FAULT_BQ;
    }
    if active {
        value |= sbits::ACTIVE_BQ;
    }
    state_bits::update_flags(MASK, value);
}

pub struct Bq76920TaskArgs {
    pub i2c_bus: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
    >,
    pub address: u8,
    pub sense_resistor_m_ohm: u32,
    pub ntc_params: Option<NtcParameters>,
    pub bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    pub bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>,
    pub sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    pub balancing_cv_publisher: BalancingCvRequestPublisher<'static>,
    /// Optional SMBus Alert (PB5) GPIO used to signal temperature events.
    pub temp_alert_pin: Option<Output<'static>>,
}

// BQ ALERT EXTI is handled in irq_mux::irq_mux_task

// Smart cell balancing logic based on charging status and voltage thresholds
async fn execute_smart_battery_balancing<'a>(
    bq: &'a mut Bq769x0<
        I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
        >,
        bq769x0_async_rs::Enabled,
        5,
    >,
    latest_core_measurements: &'a Option<bq769x0_async_rs::data_types::Bq76920Measurements<5>>,
    active_cell: &mut Option<usize>,
) {
    if let Some(measurements) = latest_core_measurements {
        let v = measurements.cell_voltages.voltages;
        // Compute spread and adjacent diffs
        let mut min_v = i32::MAX;
        let mut max_v = i32::MIN;
        let mut max_indices: heapless::Vec<usize, 5> = heapless::Vec::new();
        for (i, &vi) in v.iter().enumerate() {
            if vi <= 0 {
                continue;
            }
            if vi < min_v {
                min_v = vi;
            }
            if vi > max_v {
                max_v = vi;
                max_indices.clear();
                let _ = max_indices.push(i);
            } else if vi == max_v {
                let _ = max_indices.push(i);
            }
        }
        if max_v == i32::MIN || min_v == i32::MAX {
            defmt::debug!("No valid cell voltages available for balancing");
            let _ = bq.set_cell_balancing(0).await;
            *active_cell = None;
            return;
        }

        let spread = max_v - min_v;

        // Stop condition: all adjacent diffs <= LOCAL_PEAK_MARGIN_MV
        let mut all_adjacent_within_margin = true;
        for i in 0..4 {
            let d = (v[i] - v[i + 1]).abs();
            if d > LOCAL_PEAK_MARGIN_MV {
                all_adjacent_within_margin = false;
                break;
            }
        }
        if all_adjacent_within_margin {
            if active_cell.is_some() {
                debug!("bal:stop adj<= {}mV", LOCAL_PEAK_MARGIN_MV);
            }
            let _ = bq.set_cell_balancing(0).await;
            *active_cell = None;
            return;
        }

        // Choose a target: one of the max-voltage cells that is higher than at least one neighbor by > LOCAL_PEAK_MARGIN_MV
        let mut candidate: Option<usize> = None;
        for &i in max_indices.iter() {
            let higher_than_left = if i > 0 {
                v[i] - v[i - 1] > LOCAL_PEAK_MARGIN_MV
            } else {
                false
            };
            let higher_than_right = if i < 4 {
                v[i] - v[i + 1] > LOCAL_PEAK_MARGIN_MV
            } else {
                false
            };
            if (i == 0 && higher_than_right)
                || (i == 4 && higher_than_left)
                || (i > 0 && i < 4 && (higher_than_left || higher_than_right))
            {
                candidate = Some(i);
                break; // pick the first max that satisfies; max_v ensures it's globally max
            }
        }

        if let Some(cell_idx) = candidate {
            let mask = 1u16 << cell_idx;
            if *active_cell != Some(cell_idx) {
                if let Some(prev) = *active_cell {
                    // Ensure single-cell-only: turn all off, wait deadtime, then enable new cell
                    let _ = bq.set_cell_balancing(0).await;
                    info!(
                        "bal~ {}ms {}>{}",
                        BALANCE_SWITCH_DEADTIME_MS,
                        prev + 1,
                        cell_idx + 1
                    );
                    Timer::after(Duration::from_millis(BALANCE_SWITCH_DEADTIME_MS)).await;
                }
                info!("bal+ c{} {} d{}", cell_idx + 1, v[cell_idx], spread);
            }
            if let Err(_e) = bq.set_cell_balancing(mask).await {
                error!("bal:en!");
            } else {
                *active_cell = Some(cell_idx);
                // Read-back verification
                match bq.read_register(Register::CELLBAL1).await {
                    Ok(bits) => {
                        if (bits as u16 & mask) == 0 {
                            defmt::debug!("bal:vr w={} r={}", mask, bits);
                        }
                    }
                    Err(_e) => defmt::debug!("bal:vr err"),
                }
            }
        } else {
            // No eligible local peak at global max; disable for now
            if active_cell.is_some() {
                info!("bal:no-peak");
            }
            let _ = bq.set_cell_balancing(0).await;
            *active_cell = None;
            // Verify bits cleared
            match bq.read_register(Register::CELLBAL1).await {
                Ok(bits) => {
                    if bits != 0 {
                        defmt::debug!("bal:vr0 r={}", bits);
                    }
                }
                Err(_e) => defmt::debug!("bal:vr rd!"),
            }
        }
    } else if active_cell.is_some() {
        info!("bal:no-meas disable");
        let _ = bq.set_cell_balancing(0).await;
        *active_cell = None;
    }
}

/// Embassy task for managing the BQ76920 battery monitor IC.
///
/// This task is responsible for:
/// 1. Initializing the BQ76920 chip with a defined battery configuration.
///    This includes setting protection parameters (overvoltage, undervoltage, overcurrent).
/// 2. Critically, verifying that the applied configuration has been correctly written to the chip
///    by reading back key safety-related registers. This is done using `try_apply_config`.
/// 3. If configuration is successful and verified, enabling the Charge (CHG) and Discharge (DSG) FETs.
///    If verification fails, FETs are NOT enabled to prevent unsafe operation.
/// 4. In a continuous loop:
///    - Reading various measurements from the BQ76920:
///      - Individual cell voltages.
///      - Total pack voltage.
///      - Temperature sensor readings.
///      - Current (via Coulomb Counter).
///      - System status flags (e.g., OV, UV, SCD, OCD alerts).
///      - MOS FET status (CHG_ON, DSG_ON).
///    - Clearing any set status flags in the BQ76920.
///    - Publishing the collected alert information (system status) via `bq76920_alerts_publisher`.
///    - Publishing the comprehensive measurement data via `bq76920_measurements_publisher`.
///
/// # Arguments
///
/// * `i2c_bus`: A shared I2C bus device for communication with the BQ76920.
/// * `address`: The I2C address of the BQ76920 chip.
/// * `bq76920_alerts_publisher`: Publisher for sending BQ76920 alert data.
/// * `bq76920_measurements_publisher`: Publisher for sending BQ76920 measurement data.
///   The const generic `5` indicates the number of cells, matching the `N` for `Bq769x0`.
#[embassy_executor::task]
pub async fn bq76920_task(args: Bq76920TaskArgs) {
    let Bq76920TaskArgs {
        i2c_bus,
        address,
        sense_resistor_m_ohm,
        ntc_params,
        bq76920_alerts_publisher,
        bq76920_measurements_publisher,
        mut sc8815_alerts_subscriber,
        balancing_cv_publisher,
        temp_alert_pin,
    } = args;
    // dbg: test_fets_off
    // Initialize the BQ769x0 driver instance with CRC enabled and for 5 cells.
    // sense_resistor_m_ohm and ntc_params are now passed as arguments to this task.
    let mut bq: Bq769x0<
        I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
        >,
        bq769x0_async_rs::Enabled,
        5,
    > = Bq769x0::new(i2c_bus, address, sense_resistor_m_ohm, ntc_params);

    // Variables to store the latest readings from the sub-module, which are now in physical units.
    #[allow(unused_assignments)]
    let mut latest_core_measurements: Option<
        bq769x0_async_rs::data_types::Bq76920Measurements<5>,
    > = None;

    // use top-level BQ_ALERT_PENDING (set by IRQ task)

    // --- BQ76920 Initialization Sequence ---

    // Note: Waking the BQ76920 from SHIP mode (if it was in that mode)
    // is typically handled by external hardware, e.g., by pulling the TS1 pin high.
    // This task assumes the chip is already in NORMAL mode or has been woken up by such means.

    // Define the battery configuration.
    // Start with default values and then override specific parameters.
    // Define the battery configuration using struct update syntax.
    // `sense_resistor_uohms` is defined earlier in the function.
    let battery_config = BatteryConfig {
        // Per-cell thresholds
        overvoltage_trip: 3650u32,  // 3.65V OV per cell
        undervoltage_trip: 2500u32, // 2.50V UV per cell
        protection_config: ProtectionConfig {
            // 15A short-circuit, 10A overcurrent discharge
            scd_limit: 15_000i32,                         // 15_000 mA
            ocd_limit: 10_000i32,                         // 10_000 mA
            ..BatteryConfig::default().protection_config  // Keep default delays and flags
        },
        rsense: sense_resistor_m_ohm, // mΩ sense resistor (e.g., 3 mΩ)
        ..Default::default()
    };

    // Attempt to apply the configuration and, critically, verify that key safety registers
    // have been written correctly by reading them back.
    match bq.try_apply_config(&battery_config).await {
        Ok(_) => {
            crate::failsafe::set_bq_online(true);
            if TEST_FORCE_BQ_FETS_OFF {
                debug!("test:force_fets_off:init");
                let _ = bq.disable_discharging().await;
                let _ = bq.disable_charging().await;
            } else {
                // If configuration is verified, proceed to enable the Discharge FET.
                // Charge FET gating is handled by the BQ76920 and charger task; leave it enabled here.
                let _ = bq.enable_discharging().await;
            }
        }
        Err(BQ769x0Error::ConfigVerificationFailed { .. }) => {
            error!("bq:cfg_verify");
            crate::failsafe::set_bq_online(false);
        }
        Err(_e) => {
            error!("bq:cfg_apply");
            crate::failsafe::set_bq_online(false);
        }
    }

    // Runtime config (Bq76920RuntimeConfig) is no longer published from here,
    // as NTC parameters and sense resistor are now part of Bq769x0 driver initialization.

    // Main loop for continuous data acquisition and publishing.
    let mut balance_timer_counter: u32 = 0; // Counter for battery balancing frequency
    let mut cell_sample_elapsed: u32 = 0; // Cell measurement scheduler (seconds)
    let mut first_sample_pending: bool = true; // Force one immediate sample after init
    let mut active_balancing_cell: Option<usize> = None;
    let mut adapter_present: bool = false;
    let mut charger_expected: bool = false;
    let mut charger_confirmed: bool = false;
    let mut ov_pause_active: bool = false;
    let mut imbalance_pause_active: bool = false;
    let mut balance_retry_holdoff: u8 = 0; // seconds
    let mut last_cellbal_bits: u8 = 0; // hardware BAL bits snapshot (for change logging)
    let mut balancing_needed_by_delta: bool = false;
    let mut prev_balancing_needed_by_delta: bool = false;
    let mut snap_tick: u32 = 0;
    let mut fail_streak: u8 = 0; // BQ 连续通信失败计数（最小实现）
    let mut fault_bq_flag: bool = false;
    let mut last_pack_voltage_mv: i32 = PACK_CHARGE_START_THRESHOLD_MV;
    // Dropout counters omitted in this step to keep flash within limits
    let mut adapter_lost_logged: bool = false;
    // Fast-evaluation triggers tracking
    let mut prev_adapter_present: bool = false;
    let mut prev_charger_confirmed: bool = false;
    let mut last_eval_period_secs: u32 = 3600;

    // Temperature smoothing state
    let mut temp_ema_001c: i32 = 0;
    let mut temp_ema_inited: bool = false;
    // Unified thermal policy state/output (shared across iterations).
    let mut temp_policy_state: TempPolicyState = TempPolicyState::default();
    let mut temp_policy_output: TempPolicyOutput = TempPolicyOutput::default();
    // SMBus Alert GPIO (PB5) passed in from main, if available.
    let mut temp_alert_pin = temp_alert_pin;
    // Last-known pack spread computed from BQ measurements
    let mut last_delta_mv: Option<i32> = None;
    let mut last_delta_pct: Option<u8> = None;

    loop {
        // Reuse last-known spread unless a fresh sample updates it
        let mut delta_mv: Option<i32> = last_delta_mv;
        let mut delta_pct: Option<u8> = last_delta_pct;

        if crate::failsafe::is_quiesced() {
            // 静默时也不停止安全职责：不提前返回，只把 AC 视图标记为 false，允许后续以 60s 周期运行
            if let Some(_cell) = active_balancing_cell.take() {
                let _ = bq.set_cell_balancing(0).await;
            }
            if BQ_ALERT_PENDING.swap(false, portable_atomic::Ordering::Relaxed) {
                match with_timeout(Duration::from_secs(2), bq.read_all_measurements()).await {
                    Ok(Ok(core_meas)) => {
                        latest_core_measurements = Some(core_meas);
                        // mark online (no extra log)
                        let now_ms = embassy_time::Instant::now().as_millis() as u32;
                        crate::failsafe::bq_heartbeat_update(now_ms);
                        crate::failsafe::set_bq_online(true);
                        fail_streak = 0;
                        crate::failsafe::clear_pstop();
                        // Update spread cache for downstream charger logic
                        let mut min_v = i32::MAX;
                        let mut max_v = i32::MIN;
                        for &v in core_meas.cell_voltages.voltages.iter() {
                            if v > 0 {
                                min_v = min_v.min(v);
                                max_v = max_v.max(v);
                            }
                        }
                        if max_v != i32::MIN && min_v != i32::MAX {
                            let spread = max_v - min_v;
                            let pct = ((spread.saturating_mul(100)) / max_v.max(1)).clamp(0, 100);
                            last_delta_mv = Some(spread);
                            last_delta_pct = Some(pct as u8);
                        }
                        bq76920_alerts_publisher.publish_immediate(
                            crate::data_types::Bq76920Alerts {
                                system_status: core_meas.system_status,
                            },
                        );
                        bq76920_measurements_publisher.publish_immediate(
                            crate::data_types::Bq76920Measurements {
                                core_measurements: core_meas,
                            },
                        );
                        let flags_to_clear = core_meas.system_status.0.bits();
                        if flags_to_clear != 0 {
                            let _ = bq.clear_status_flags(flags_to_clear).await;
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        fail_streak = fail_streak.saturating_add(1);
                        if fail_streak >= 3 {
                            crate::failsafe::request_pstop();
                        }
                        bq76920_alerts_publisher.publish_immediate(
                            crate::data_types::Bq76920Alerts {
                                system_status: Default::default(),
                            },
                        );
                    }
                }
            }
            adapter_present = false;
            let full_flag = (state_bits::flags() & sbits::FULL) != 0;
            let preparing = !full_flag && last_pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV;
            // Preserve ACTIVE_BQ here to avoid per-second flicker on Red LED.
            // ACTIVE_BQ is decided once per tick at loop end (based on scheduler cadence).
            let prev_active = (state_bits::flags() & sbits::ACTIVE_BQ) != 0;
            update_bq_state(preparing, false, fault_bq_flag, prev_active);
            // 不再 continue；后续进入统一的周期调度
        }
        // This task focuses on reading data from the BQ76920 itself.
        // Communication with other chips (like BQ25730 charger) is handled in their respective tasks.

        // Note: The CC_EN (Coulomb Counter Enable) flag in SYS_CTRL2 is set by default
        // in `BatteryConfig::default()` and verified by `try_apply_config`.
        // Therefore, an explicit check and write for CC_EN in this loop is no longer necessary.

        if TEST_FORCE_BQ_FETS_OFF {
            // Ensure cell balancing FETs are also off during diagnostics.
            let _ = bq.set_cell_balancing(0).await;
        }

        // 采样调度："实际均衡活跃"或"充电相位"时 1s；其余按阶段 30/60s；ALERT 立即采样
        // 注：仅“需要均衡（Δcell≥阈值）”而未处于充电相位时，不再将周期强制为 1s，避免 AC 不在时的高频打点。
        let charging_phase =
            charger_expected || charger_confirmed || ov_pause_active || imbalance_pause_active;
        let hw_balancing_active_now = last_cellbal_bits != 0
            || active_balancing_cell.is_some()
            || (balancing_needed_by_delta && charging_phase);
        let period_secs: u32 = if hw_balancing_active_now {
            1
        } else if charging_phase {
            30
        } else {
            60
        };
        let bq_active_flag = period_secs < 30;
        let mut sample_due = cell_sample_elapsed >= period_secs || first_sample_pending;
        let alert_due_now = BQ_ALERT_PENDING.swap(false, portable_atomic::Ordering::Relaxed);
        if alert_due_now {
            sample_due = true;
        }

        if sample_due {
            // dbg: read begin

            // Read ADC calibration values (not used in current logging but kept for potential future use)
            let (_adc_gain_uv_per_lsb, _adc_offset_mv) = match bq.read_adc_calibration().await {
                Ok(cal) => cal,
                Err(_e) => {
                    error!("bq:adc_cal");
                    // Use default calibration values if reading fails
                    (365, 0) // Default values from datasheet
                }
            };

            // Read and display cell balancing status
            let cellbal1_register = bq.read_register(Register::CELLBAL1).await.unwrap_or(0);
            if VERBOSE_BQ_LOG {
                debug!(
                    "bal:stat 0b{:08b}(0x{:02X})",
                    cellbal1_register, cellbal1_register
                );
            }

            // Display which cells are enabled for balancing
            let mut balancing_cells = [0u8; 5];
            let mut balancing_count = 0;
            for i in 0..5 {
                if (cellbal1_register & (1 << i)) != 0 {
                    balancing_cells[balancing_count] = (i + 1) as u8;
                    balancing_count += 1;
                }
            }

            if cellbal1_register != last_cellbal_bits {
                if balancing_count == 0 {
                    debug!("bal:none");
                } else {
                    debug!("bal:{:?}", &balancing_cells[..balancing_count]);
                }
                last_cellbal_bits = cellbal1_register;
            }

            // Read all measurements from BQ76920 with a timeout, so hangs become failures.
            match with_timeout(Duration::from_secs(2), bq.read_all_measurements()).await {
                Ok(Ok(core_meas)) => {
                    latest_core_measurements = Some(core_meas);
                    fail_streak = 0;
                    crate::failsafe::clear_pstop();
                    // Heartbeat + online after a good sample
                    let now_ms = embassy_time::Instant::now().as_millis() as u32;
                    crate::failsafe::bq_heartbeat_update(now_ms);
                    crate::failsafe::set_bq_online(true);

                    // Detailed BQ76920 measurements (optional verbose)
                    // dbg: verbose block removed to save flash

                    // snapshot disabled to save flash

                    // Evaluate pack-level conditions
                    let pack_voltage_mv = core_meas.total_voltage_mv;
                    let pack_current_ma = core_meas.current_ma;

                    if LOG_CELL_DELTA {
                        // Log pack-level voltage and current at INFO level when detailed
                        // cell delta logging is enabled so that we can correlate SC8815
                        // readings, PSU input power and BQ76920 pack-side measurements.
                        info!(
                            "bq:meas vb={}mV ibat={}mA",
                            pack_voltage_mv, pack_current_ma
                        );
                    }
                    let uv_fault = core_meas.system_status.0.contains(SysStatFlags::UV);
                    let ov_fault = core_meas.system_status.0.contains(SysStatFlags::OV);
                    let scd_fault = core_meas.system_status.0.contains(SysStatFlags::SCD);
                    let ocd_fault = core_meas.system_status.0.contains(SysStatFlags::OCD);
                    fault_bq_flag = uv_fault || ov_fault || scd_fault || ocd_fault;
                    last_pack_voltage_mv = pack_voltage_mv;
                    let protection_allows_discharge = !uv_fault && !scd_fault && !ocd_fault;

                    let mut should_enable_discharge = protection_allows_discharge;

                    if pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV {
                        debug!(
                            "dsg:off {}<={}",
                            pack_voltage_mv, PACK_OUTPUT_CUTOFF_THRESHOLD_MV
                        );
                        should_enable_discharge = false;
                    }

                    // Apply unified thermal gating (previous iteration's policy
                    // output). Immediate reactions for new events are handled
                    // after the thermal evaluation near the end of the loop.
                    if !temp_policy_output.allow_discharge {
                        should_enable_discharge = false;
                    }

                    let is_discharge_currently_on =
                        core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON);

                    if TEST_FORCE_BQ_FETS_OFF {
                        if is_discharge_currently_on {
                            let _ = bq.disable_discharging().await;
                        }
                    } else if should_enable_discharge && !is_discharge_currently_on {
                        let _ = bq.enable_discharging().await;
                    } else if !should_enable_discharge && is_discharge_currently_on {
                        let _ = bq.disable_discharging().await;
                    }

                    // Compute whether balancing is needed by pack spread threshold (start condition)
                    balancing_needed_by_delta = false;
                    if let Some(meas) = latest_core_measurements.as_ref() {
                        let mut min_v = i32::MAX;
                        let mut max_v = i32::MIN;
                        for &v in meas.cell_voltages.voltages.iter() {
                            if v > 0 {
                                if v < min_v {
                                    min_v = v;
                                }
                                if v > max_v {
                                    max_v = v;
                                }
                            }
                        }
                        if max_v != i32::MIN && min_v != i32::MAX {
                            let spread = max_v - min_v;
                            let pct = ((spread.saturating_mul(100)) / max_v.max(1)).clamp(0, 100);
                            delta_mv = Some(spread);
                            delta_pct = Some(pct as u8);
                            if spread >= BALANCE_START_SPREAD_MV {
                                balancing_needed_by_delta = true;
                            }
                            if balancing_needed_by_delta != prev_balancing_needed_by_delta {
                                debug!("bal:e s={} n={}", spread, balancing_needed_by_delta);
                                prev_balancing_needed_by_delta = balancing_needed_by_delta;
                            }
                            if LOG_CELL_DELTA {
                                info!(
                                    "cell: v=[{} {} {} {} {}] min={} max={} spread={} pct={}",
                                    meas.cell_voltages.voltages[0],
                                    meas.cell_voltages.voltages[1],
                                    meas.cell_voltages.voltages[2],
                                    meas.cell_voltages.voltages[3],
                                    meas.cell_voltages.voltages[4],
                                    min_v,
                                    max_v,
                                    spread,
                                    pct
                                );
                            }
                        }
                    }

                    // Preview whether CV hold is required (balancing not complete)
                    // Use current active cell, any HW balancing bits, or spread-based need (computed below).
                    let hw_balancing_bits_active = last_cellbal_bits != 0;
                    let require_cv_preview = adapter_present
                        && (active_balancing_cell.is_some()
                            || hw_balancing_bits_active
                            || balancing_needed_by_delta);

                    let mut should_enable_charging =
                        !ov_fault && !scd_fault && !ocd_fault && !uv_fault;

                    if (pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV && !require_cv_preview)
                        || pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV
                    {
                        should_enable_charging = false;
                    }

                    // Thermal policy may further restrict charging.
                    if !temp_policy_output.allow_charge {
                        should_enable_charging = false;
                    }

                    if should_enable_charging {
                        let ctrl = charger_control::snapshot();
                        if !ctrl.auto_enabled && !ctrl.manual_enable {
                            should_enable_charging = false;
                        }
                    }

                    if TEST_FORCE_BQ_FETS_OFF {
                        let _ = bq.disable_charging().await;
                        debug!("TEST: Forcing BQ76920 CHG/DSG OFF (runtime)");
                    } else if should_enable_charging {
                        let _ = bq.enable_charging().await;
                    } else {
                        let _ = bq.disable_charging().await;
                    }

                    // Publish BQ76920 alert information (derived from system status).
                    let alerts = crate::data_types::Bq76920Alerts {
                        system_status: core_meas.system_status,
                    };
                    bq76920_alerts_publisher.publish_immediate(alerts);

                    // It's important to clear any set status flags after reading them,
                    // so that new events can be detected. Writing '1' to a bit clears it.
                    let flags_to_clear = core_meas.system_status.0.bits();
                    if flags_to_clear != 0 {
                        if let Err(_e_clear) = bq.clear_status_flags(flags_to_clear).await {
                            error!("bq:clr_err");
                        } else { /* dbg clr */
                        }
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    error!("bq:meas!");
                    latest_core_measurements = None;
                    fail_streak = fail_streak.saturating_add(1);
                    warn!("bq:fail_streak={}", fail_streak);
                    if fail_streak >= 3 {
                        crate::failsafe::set_bq_online(false);
                    }
                    if fail_streak >= 3 {
                        crate::failsafe::request_pstop();
                    }
                    // Optionally publish default/error state for alerts if needed
                    let alerts = crate::data_types::Bq76920Alerts {
                        system_status: Default::default(),
                    };
                    bq76920_alerts_publisher.publish_immediate(alerts);
                }
            }

            // last_cellbal_bits already updated earlier on change; keep it as snapshot

            // Compute and log temperature (EMA when active; median-of-3 when inactive)
            let mut t_bq_int_0_01c = TEMP_INVALID_0_01C;
            if let Some(core) = latest_core_measurements.as_ref() {
                // Choose internal TS1 (die temperature) per project requirement
                let raw_t_001c: i16 = core.temperatures.ts1;
                let bq_active_flag = period_secs < 30;
                // Will be assigned by EMA (active) or median-of-3 (inactive) branches below
                let used_t_001c: i16;
                if bq_active_flag {
                    // EMA smoothing
                    if !temp_ema_inited {
                        temp_ema_001c = i32::from(raw_t_001c);
                        temp_ema_inited = true;
                    } else {
                        // ema = ema*(1-a) + x*a; all in 0.01°C integer domain
                        let a = TEMP_EMA_ALPHA_PCT;
                        temp_ema_001c =
                            (temp_ema_001c * (100 - a) + i32::from(raw_t_001c) * a + 50) / 100;
                    }
                    used_t_001c = temp_ema_001c as i16;
                    defmt::debug!("bq:t= {} (ema) raw={} (0.01C)", used_t_001c, raw_t_001c);
                } else {
                    // Median of three consecutive reads (temperature only) when inactive
                    let mut buf = [raw_t_001c, raw_t_001c, raw_t_001c];
                    // Two additional quick samples
                    if let Ok(t2) = bq.read_temperatures().await.map(|t| t.ts1) {
                        buf[1] = t2;
                    }
                    Timer::after(Duration::from_millis(5)).await;
                    if let Ok(t3) = bq.read_temperatures().await.map(|t| t.ts1) {
                        buf[2] = t3;
                    }
                    // sort 3 values to get median
                    if buf[0] > buf[1] {
                        buf.swap(0, 1);
                    }
                    if buf[1] > buf[2] {
                        buf.swap(1, 2);
                    }
                    if buf[0] > buf[1] {
                        buf.swap(0, 1);
                    }
                    used_t_001c = buf[1];
                    // keep EMA aligned to latest value when inactive
                    temp_ema_001c = i32::from(used_t_001c);
                    temp_ema_inited = true;
                    defmt::debug!(
                        "bq:t= {} (med3) r0={} r1={} r2={} (0.01C)",
                        used_t_001c,
                        buf[0],
                        buf[1],
                        buf[2]
                    );
                }

                // Publish filtered BQ internal temperature into thermal aggregation.
                t_bq_int_0_01c = used_t_001c;
            }
            // If we have no valid core measurements, t_bq_int_0_01c stays INVALID.
            thermal::update_bq_int_temp(t_bq_int_0_01c);

            // Evaluate unified thermal policy based on the latest aggregated snapshot.
            let snapshot = thermal::snapshot();
            let (new_policy_state, new_policy_output) =
                temp_policy::eval(&temp_policy_state, &snapshot);
            temp_policy_state = new_policy_state;
            temp_policy_output = new_policy_output;

            // Mirror TEMP_STATUS onto the I2C register map.
            i2c_slave::update_temp_status(temp_policy_output.temp_status_bits);

            // Drive SMBus Alert (PB5) when high-temperature protection is active
            // (TEMP_HIGH_CHG or TEMP_HIGH_DSG). Low-temperature only is conveyed
            // via TEMP_STATUS without asserting the alert line.
            let high_temp_bits = temp_policy_output.temp_status_bits
                & (temp_policy::bits::TEMP_HIGH_CHG | temp_policy::bits::TEMP_HIGH_DSG);
            if let Some(pin) = temp_alert_pin.as_mut() {
                if high_temp_bits != 0 {
                    pin.set_low();
                } else {
                    pin.set_high();
                }
            }

            // Enforce CHG/DSG thermal gating immediately when protections are active.
            if let Some(core) = latest_core_measurements.as_ref() {
                let mos_status = core.mos_status.0;
                let dsg_on = mos_status.contains(SysCtrl2Flags::DSG_ON);
                let chg_on = mos_status.contains(SysCtrl2Flags::CHG_ON);

                if !temp_policy_output.allow_discharge && dsg_on {
                    let _ = bq.disable_discharging().await;
                }
                if !temp_policy_output.allow_charge && chg_on {
                    let _ = bq.disable_charging().await;
                }
            }

            // Treat a hard discharge over-temperature trip as a BQ fault for the
            // system-level STATE_FLAGS.
            if (temp_policy_output.temp_status_bits & temp_policy::bits::TEMP_HIGH_DSG) != 0 {
                fault_bq_flag = true;
            }

            // 发布测量（即便失败也发布默认值，便于外设镜像）
            let bq76920_measurements_payload_for_main_pub =
                crate::data_types::Bq76920Measurements {
                    core_measurements: latest_core_measurements.unwrap_or_default(),
                };
            bq76920_measurements_publisher
                .publish_immediate(bq76920_measurements_payload_for_main_pub);
            // Keep last-known spread for charger-side policies even if the next tick lacks a fresh sample
            last_delta_mv = delta_mv;
            last_delta_pct = delta_pct;
            if latest_core_measurements.is_some() {
                crate::failsafe::set_bq_online(true);
                fail_streak = 0;
            }

            cell_sample_elapsed = 0;
            first_sample_pending = false;
        } // end sample_due

        // (spread re-evaluated earlier within the OK-measurements branch)

        // （按需）其余逻辑基于最新一次有效测量

        // Pull latest charger/adapter alerts
        if let Some(sc_alerts) = sc8815_alerts_subscriber.try_next_message_pure() {
            adapter_present = sc_alerts.device_status.ac_adapter_connected;
            charger_expected = sc_alerts.expected_charging;
            charger_confirmed = sc_alerts.charging_confirmed;
            ov_pause_active = sc_alerts.ov_pause_active;
            imbalance_pause_active = sc_alerts.imbalance_pause_active;
        }

        // Treat charge or charge-pause as charging cadence, evaluated only together with AC presence below.
        // Note: we keep pause flags here for cadence, but AC presence will hard-gate balancing.
        let charging_phase =
            charger_expected || charger_confirmed || ov_pause_active || imbalance_pause_active;
        let eval_period_secs: u32 = 1;
        if eval_period_secs != last_eval_period_secs {
            info!(
                "bal:eval={} ac={} chgph={}",
                eval_period_secs, adapter_present, charging_phase
            );
            last_eval_period_secs = eval_period_secs;
        }

        // Strict policy: if adapter is absent, balancing must not be active under any circumstance.
        if !adapter_present && last_cellbal_bits != 0 {
            defmt::debug!("bal:stop no-ac hw=0x{:02X}", last_cellbal_bits);
            let _ = bq.set_cell_balancing(0).await;
            active_balancing_cell = None;
            last_cellbal_bits = 0;
        }

        // Determine if we should request CV hold from charger
        let hw_balancing_active = last_cellbal_bits != 0;
        // Charger should hold CV when balancing is required or active, but only when AC is present
        // and the unified thermal policy allows balancing.
        let mut require_cv = adapter_present
            && temp_policy_output.allow_balancing
            && (active_balancing_cell.is_some()
                || hw_balancing_active
                || balancing_needed_by_delta);
        // LED overlay仅在“硬件正在均衡”时显示（避免仅因"require_cv"而产生抖动观感）。
        let overlay_led = active_balancing_cell.is_some() || hw_balancing_active;

        // Rising-edge based fast-evaluation triggers (kept for logs; evaluation is every second)
        let adapter_rising = adapter_present && !prev_adapter_present;
        let charge_rising = charger_confirmed && !prev_charger_confirmed;

        // --- Battery Balancing Logic (periodic + fast triggers) ---
        let periodic_due = balance_timer_counter == 0 || balance_timer_counter >= eval_period_secs;
        let fast_due = adapter_rising || charge_rising;
        if periodic_due || fast_due {
            // 3600 seconds = 1 hour
            if !TEST_FORCE_BQ_FETS_OFF {
                // Strict policy: balancing only allowed when adapter is present.
                // Additionally, never allow balancing when the thermal policy
                // has disabled it.
                let balancing_env = adapter_present && temp_policy_output.allow_balancing;
                if balancing_env && charging_phase {
                    execute_smart_battery_balancing(
                        &mut bq,
                        &latest_core_measurements,
                        &mut active_balancing_cell,
                    )
                    .await;
                }
            } else {
                // During diagnostics, keep all balancing off.
                if active_balancing_cell.is_some() {
                    let _ = bq.set_cell_balancing(0).await;
                    active_balancing_cell = None;
                }
            }
            balance_timer_counter = 0; // Reset counter after execution
        }
        // --- End Battery Balancing Logic ---

        // If temperature pause becomes active while balancing, stop immediately.
        if !temp_policy_output.allow_balancing
            && (active_balancing_cell.is_some() || last_cellbal_bits != 0)
        {
            defmt::debug!("bal:stop temp_protect hw=0x{:02X}", last_cellbal_bits);
            let _ = bq.set_cell_balancing(0).await;
            active_balancing_cell = None;
            last_cellbal_bits = 0;
            // During temp pause we also withdraw CV request to avoid keeping charge path primed.
            require_cv = false;
        }

        // If adapter lost while balancing, stop immediately (log once until recovery)
        // 在暂停环境下（OV/严重不均衡），即使 adapter 不在也不立即停均衡
        if active_balancing_cell.is_some()
            && !adapter_present
            && !ov_pause_active
            && !imbalance_pause_active
        {
            if !adapter_lost_logged {
                info!("bal:lost-ac stop & withdraw CV");
                adapter_lost_logged = true;
            }
            let _ = bq.set_cell_balancing(0).await;
            active_balancing_cell = None;
            require_cv = false;
            balance_retry_holdoff = balance_retry_holdoff.max(5);
        }
        if adapter_present {
            adapter_lost_logged = false; // reset latch on recovery
        }

        // Publish the coupling signal each tick
        // Severe imbalance flag (Δ>=100 mV)
        let mut severe_imbalance_flag = delta_mv.map(|d| d >= 100).unwrap_or(false);
        if !severe_imbalance_flag {
            if let Some(meas) = latest_core_measurements.as_ref() {
                let mut min_v = i32::MAX;
                let mut max_v = i32::MIN;
                for &v in meas.cell_voltages.voltages.iter() {
                    if v > 0 {
                        if v < min_v {
                            min_v = v;
                        }
                        if v > max_v {
                            max_v = v;
                        }
                    }
                }
                if max_v != i32::MIN && min_v != i32::MAX {
                    severe_imbalance_flag = (max_v - min_v) >= 100;
                }
            }
        }

        balancing_cv_publisher.publish_immediate(BalancingCvRequest {
            require_cv,
            overlay: overlay_led,
            severe_imbalance: severe_imbalance_flag,
            // Request SC-side temperature pause whenever charge is thermally
            // disallowed by the unified policy.
            temp_pause: !temp_policy_output.allow_charge,
            delta_mv,
            delta_pct,
        });

        // dbg: read end

        // Wait for a defined interval before the next cycle of readings.
        Timer::after(Duration::from_secs(1)).await;
        balance_timer_counter += 1;
        cell_sample_elapsed = cell_sample_elapsed.saturating_add(1);
        snap_tick = snap_tick.wrapping_add(1);
        let prev_holdoff = balance_retry_holdoff;
        if balance_retry_holdoff > 0 {
            balance_retry_holdoff = balance_retry_holdoff.saturating_sub(1);
        }
        // 当抑制计时刚好归零时，立刻执行一次快速评估，避免再等到下一周期
        if prev_holdoff > 0
            && balance_retry_holdoff == 0
            && adapter_present
            && !TEST_FORCE_BQ_FETS_OFF
        {
            info!("bal:holdoff");
            execute_smart_battery_balancing(
                &mut bq,
                &latest_core_measurements,
                &mut active_balancing_cell,
            )
            .await;
        }

        let full_flag = (state_bits::flags() & sbits::FULL) != 0;
        let preparing =
            !adapter_present && !full_flag && last_pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV;
        let balancing_flag = last_cellbal_bits != 0 || active_balancing_cell.is_some();
        update_bq_state(preparing, balancing_flag, fault_bq_flag, bq_active_flag);

        // Update edge tracking
        prev_adapter_present = adapter_present;
        prev_charger_confirmed = charger_confirmed;
    }
}
