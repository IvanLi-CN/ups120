use bq769x0_async_rs::RegisterAccess;
use bq769x0_async_rs::registers::{Register, SysCtrl2Flags, SysStatFlags};
use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq769x0_async_rs::ProtectionConfig;
use bq769x0_async_rs::{
    BatteryConfig, Bq769x0, data_types::NtcParameters, errors::Error as BQ769x0Error,
};

// Import necessary data types
use crate::data_types::BalancingCvRequest;
use crate::shared::{
    BalancingCvRequestPublisher, Bq76920AlertsPublisher, Bq76920MeasurementsPublisher,
    Sc8815AlertsSubscriber,
};

const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_OUTPUT_CUTOFF_THRESHOLD_MV: i32 = 12_500;
// New balancing policy thresholds
// Start balancing during charging when pack spread (max-min) exceeds 10 mV.
const BALANCE_START_SPREAD_MV: i32 = 10;
// A local peak must exceed at least one adjacent cell by >1 mV to be eligible.
const LOCAL_PEAK_MARGIN_MV: i32 = 1;

// Logging verbosity toggles for BQ76920 task
const VERBOSE_BQ_LOG: bool = false; // set true for full register-by-register dumps
const SNAP_BQ_EVERY_SEC: u32 = 0; // disable one-line snapshot

// Test knob: force both CHG/DSG FETs off for charger-path diagnostics.
// Default false for normal operation; set true only for lab diagnostics.
const TEST_FORCE_BQ_FETS_OFF: bool = false;
// Interlock deadtime when switching balancing target cells (safety)
const BALANCE_SWITCH_DEADTIME_MS: u64 = 40;

// Smart cell balancing logic based on charging status and voltage thresholds
async fn execute_smart_battery_balancing<'a>(
    bq: &'a mut Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>>,
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
            warn!("No valid cell voltages available for balancing");
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
            if active_cell.is_some() { debug!("bal:stop adj<= {}mV", LOCAL_PEAK_MARGIN_MV); }
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
                    debug!("bal~ {}ms {}>{}", BALANCE_SWITCH_DEADTIME_MS, prev+1, cell_idx+1);
                    Timer::after(Duration::from_millis(BALANCE_SWITCH_DEADTIME_MS)).await;
                }
                debug!("bal+ c{} {} d{}", cell_idx+1, v[cell_idx], spread);
            }
            if let Err(_e) = bq.set_cell_balancing(mask).await {
                error!("bal:en!");
            } else {
                *active_cell = Some(cell_idx);
                // Read-back verification
                match bq.read_register(Register::CELLBAL1).await {
                    Ok(bits) => {
                        if (bits as u16 & mask) == 0 {
                            warn!("bal:vr w={} r={}", mask, bits);
                        }
                    }
                    Err(_e) => warn!("bal:vr err"),
                }
            }
        } else {
            // No eligible local peak at global max; disable for now
            if active_cell.is_some() {
                debug!("bal:no-peak");
            }
            let _ = bq.set_cell_balancing(0).await;
            *active_cell = None;
            // Verify bits cleared
            match bq.read_register(Register::CELLBAL1).await {
                Ok(bits) => {
                    if bits != 0 {
                        warn!("bal:vr0 r={}", bits);
                    }
                }
                Err(_e) => warn!("bal:vr rd!"),
            }
        }
    } else {
        if active_cell.is_some() {
            debug!("bal:no-meas disable");
            let _ = bq.set_cell_balancing(0).await;
            *active_cell = None;
        }
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
pub async fn bq76920_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>>,
    address: u8,
    sense_resistor_m_ohm: u32, // Added: Sense resistor value in mOhms
    ntc_params: Option<NtcParameters>, // Added: NTC parameters
    bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    balancing_cv_publisher: BalancingCvRequestPublisher<'static>,
) {
    debug!("test_fets_off={}", TEST_FORCE_BQ_FETS_OFF);
    // Initialize the BQ769x0 driver instance with CRC enabled and for 5 cells.
    // sense_resistor_m_ohm and ntc_params are now passed as arguments to this task.
    let mut bq: Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>>,
        bq769x0_async_rs::Enabled,
        5,
    > = Bq769x0::new(i2c_bus, address, sense_resistor_m_ohm, ntc_params);

    // Variables to store the latest readings from the sub-module, which are now in physical units.
    #[allow(unused_assignments)]
    let mut latest_core_measurements: Option<
        bq769x0_async_rs::data_types::Bq76920Measurements<5>,
    > = None;

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
        Err(BQ769x0Error::ConfigVerificationFailed { .. }) => { error!("bq:cfg_verify"); }
        Err(_e) => { error!("bq:cfg_apply"); }
    }

    // Runtime config (Bq76920RuntimeConfig) is no longer published from here,
    // as NTC parameters and sense resistor are now part of Bq769x0 driver initialization.

    // Main loop for continuous data acquisition and publishing.
    let mut balance_timer_counter: u32 = 0; // Counter for battery balancing frequency
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
    let mut adapter_lost_logged: bool = false;
    // Fast-evaluation triggers tracking
    let mut prev_adapter_present: bool = false;
    let mut prev_charger_confirmed: bool = false;
    let mut last_eval_period_secs: u32 = 3600;

    loop {
        if crate::scheduler::is_quiesced() {
            // Minimal maintenance: ensure balancing off and avoid polling.
            if let Some(cell) = active_balancing_cell.take() {
                let _ = bq.set_cell_balancing(0).await;
                let _ = cell; // silence unused
            }
            // Block until system becomes active again; no periodic timers while quiesced
            crate::scheduler::wait_until_active().await;
            continue;
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

        if VERBOSE_BQ_LOG { debug!("bq:read"); }

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
        if VERBOSE_BQ_LOG { debug!("bal:stat 0b{:08b}(0x{:02X})", cellbal1_register, cellbal1_register); }

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
            if balancing_count == 0 { debug!("bal:none"); } else { debug!("bal:{:?}", &balancing_cells[..balancing_count]); }
            last_cellbal_bits = cellbal1_register;
        }

        // Read all measurements from BQ76920. These are now in physical units.
        match bq.read_all_measurements().await {
            Ok(core_meas) => {
                latest_core_measurements = Some(core_meas);

                // Detailed BQ76920 measurements (optional verbose)
                if VERBOSE_BQ_LOG {
                    debug!("cells:");
                    for i in 0..5 { let v = core_meas.cell_voltages.voltages[i]; debug!("c{}={}mV", i+1, v); }
                    debug!("pack={}mV cur={}mA", core_meas.total_voltage_mv, core_meas.current_ma);
                    let ts1 = core_meas.temperatures.ts1; debug!("ts1={}c1e-2", ts1);
                    if let Some(ts2) = core_meas.temperatures.ts2 { debug!("ts2={}c1e-2", ts2); }
                    if let Some(ts3) = core_meas.temperatures.ts3 { debug!("ts3={}c1e-2", ts3); }
                    debug!("sys=0x{:02X}", core_meas.system_status.0.bits());
                    debug!("mos chg={} dsg={} cc={} oneshot={} dly={}",
                        core_meas.mos_status.0.contains(SysCtrl2Flags::CHG_ON),
                        core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON),
                        core_meas.mos_status.0.contains(SysCtrl2Flags::CC_EN),
                        core_meas.mos_status.0.contains(SysCtrl2Flags::CC_ONESHOT),
                        core_meas.mos_status.0.contains(SysCtrl2Flags::DELAY_DIS));
                }

                // One-line snapshot (always enabled for quick diagnostics)
                if SNAP_BQ_EVERY_SEC > 0 && (snap_tick % SNAP_BQ_EVERY_SEC == 0) {
                    let chg_on = core_meas.mos_status.0.contains(SysCtrl2Flags::CHG_ON);
                    let dsg_on = core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON);
                    let cells = core_meas.cell_voltages.voltages;
                    // TS1 is in 0.01°C units; avoid floats in no_std
                    let ts1_centi = core_meas.temperatures.ts1;
                    let ts1_i = ts1_centi / 100;
                    let mut ts1_f = ts1_centi % 100;
                    if ts1_f < 0 {
                        ts1_f = -ts1_f;
                    }
                    // Critical fault summary (mask out non-fault bits like CC_READY)
                    let fault_mask = (SysStatFlags::UV
                        | SysStatFlags::OV
                        | SysStatFlags::SCD
                        | SysStatFlags::OCD)
                        .bits();
                    let faults = core_meas.system_status.0.bits() & fault_mask;
                    // Derive current balancing cell number (0 if none)
                    let mut bal_cell_num: u8 = 0;
                    if last_cellbal_bits != 0 {
                        for i in 0..5 {
                            if (last_cellbal_bits & (1 << i)) != 0 {
                                bal_cell_num = (i + 1) as u8;
                                break;
                            }
                        }
                    }
                    // Build a human-readable fault list
                    let mut faults_str: heapless::String<24> = heapless::String::new();
                    if faults == 0 {
                        let _ = core::fmt::Write::write_str(&mut faults_str, "none");
                    } else {
                        let mut first = true;
                        let append = |s: &str, out: &mut heapless::String<24>, first: &mut bool| {
                            if !*first {
                                let _ = core::fmt::Write::write_str(out, "|");
                            }
                            let _ = core::fmt::Write::write_str(out, s);
                            *first = false;
                        };
                        if (faults & SysStatFlags::UV.bits()) != 0 {
                            append("UV", &mut faults_str, &mut first);
                        }
                        if (faults & SysStatFlags::OV.bits()) != 0 {
                            append("OV", &mut faults_str, &mut first);
                        }
                        if (faults & SysStatFlags::OCD.bits()) != 0 {
                            append("OCD", &mut faults_str, &mut first);
                        }
                        if (faults & SysStatFlags::SCD.bits()) != 0 {
                            append("SCD", &mut faults_str, &mut first);
                        }
                    }
                    info!(
                        "BQ snap: pack={}mV curr={}mA ts1={}.{}C chg={} dsg={} faults=0x{:02X}({}) bal_cell={} cells=[{},{},{},{},{}]mV",
                        core_meas.total_voltage_mv,
                        core_meas.current_ma,
                        ts1_i,
                        ts1_f,
                        chg_on,
                        dsg_on,
                        faults,
                        faults_str.as_str(),
                        bal_cell_num,
                        cells[0],
                        cells[1],
                        cells[2],
                        cells[3],
                        cells[4]
                    );
                }

                // Evaluate pack-level conditions
                let pack_voltage_mv = core_meas.total_voltage_mv;
                let uv_fault = core_meas.system_status.0.contains(SysStatFlags::UV);
                let ov_fault = core_meas.system_status.0.contains(SysStatFlags::OV);
                let scd_fault = core_meas.system_status.0.contains(SysStatFlags::SCD);
                let ocd_fault = core_meas.system_status.0.contains(SysStatFlags::OCD);
                let protection_allows_discharge = !uv_fault && !scd_fault && !ocd_fault;

                let mut should_enable_discharge = protection_allows_discharge;

                if pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV {
                    debug!("dsg:off {}<={}", pack_voltage_mv, PACK_OUTPUT_CUTOFF_THRESHOLD_MV);
                    should_enable_discharge = false;
                }

                let is_discharge_currently_on =
                    core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON);

                if TEST_FORCE_BQ_FETS_OFF {
                    if is_discharge_currently_on {
                        let _ = bq.disable_discharging().await;
                    }
                } else {
                    if should_enable_discharge && !is_discharge_currently_on {
                        let _ = bq.enable_discharging().await;
                    } else if !should_enable_discharge && is_discharge_currently_on {
                        let _ = bq.disable_discharging().await;
                    }
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
                        if spread >= BALANCE_START_SPREAD_MV {
                            balancing_needed_by_delta = true;
                        }
                        if balancing_needed_by_delta != prev_balancing_needed_by_delta {
                            info!("bal:e s={} n={}", spread, balancing_needed_by_delta);
                            prev_balancing_needed_by_delta = balancing_needed_by_delta;
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

                let mut should_enable_charging = !ov_fault && !scd_fault && !ocd_fault && !uv_fault;

                if (pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV && !require_cv_preview)
                    || pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV
                {
                    should_enable_charging = false;
                }

                if TEST_FORCE_BQ_FETS_OFF {
                    let _ = bq.disable_charging().await;
                    info!("TEST: Forcing BQ76920 CHG/DSG OFF (runtime)");
                } else {
                    if should_enable_charging {
                        let _ = bq.enable_charging().await;
                    } else {
                        let _ = bq.disable_charging().await;
                    }
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
                    if let Err(e_clear) = bq.clear_status_flags(flags_to_clear).await {
                        error!("bq:clr_err {:?}", e_clear);
                    } else {
                        debug!("bq:clr {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(_e) => {
                error!("bq:meas!");
                latest_core_measurements = None;
                // Optionally publish default/error state for alerts if needed
                let alerts = crate::data_types::Bq76920Alerts::default();
                bq76920_alerts_publisher.publish_immediate(alerts);
            }
        }

        // last_cellbal_bits already updated earlier on change; keep it as snapshot

        // (spread re-evaluated earlier within the OK-measurements branch)

        // Construct the BQ76920 measurements payload for the main `AllMeasurements` publisher.
        // If read_all_measurements failed, use default values.
        let bq76920_measurements_payload_for_main_pub = crate::data_types::Bq76920Measurements {
            core_measurements: latest_core_measurements.unwrap_or_default(),
        };

        // Publish the collected BQ76920 measurements (which are now wrapped in the main project's type).
        bq76920_measurements_publisher.publish_immediate(bq76920_measurements_payload_for_main_pub);

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
            debug!("bal:per={} ac={} chgph={}", eval_period_secs, adapter_present, charging_phase);
            last_eval_period_secs = eval_period_secs;
        }

        // Strict policy: if adapter is absent, balancing must not be active under any circumstance.
        if !adapter_present && last_cellbal_bits != 0 {
            warn!("bal:stop no-ac hw=0x{:02X}", last_cellbal_bits);
            let _ = bq.set_cell_balancing(0).await;
            active_balancing_cell = None;
            last_cellbal_bits = 0;
        }

        // Determine if we should request CV hold from charger
        let hw_balancing_active = last_cellbal_bits != 0;
        // Charger should hold CV when balancing is required or active, but only when AC is present.
        let mut require_cv = adapter_present
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
                let balancing_env = adapter_present;
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

        // If adapter lost while balancing, stop immediately (log once until recovery)
        // 在暂停环境下（OV/严重不均衡），即使 adapter 不在也不立即停均衡
        if active_balancing_cell.is_some()
            && !adapter_present
            && !ov_pause_active
            && !imbalance_pause_active
        {
            if !adapter_lost_logged {
            debug!("bal:lost-ac stop & withdraw CV");
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
        let mut severe_imbalance_flag = false;
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

        balancing_cv_publisher.publish_immediate(BalancingCvRequest {
            require_cv,
            overlay: overlay_led,
            severe_imbalance: severe_imbalance_flag,
        });

        if VERBOSE_BQ_LOG { debug!("bq:rd end"); }

        // Wait for a defined interval before the next cycle of readings.
        Timer::after(Duration::from_secs(1)).await;
        balance_timer_counter += 1;
        snap_tick = snap_tick.wrapping_add(1);
        let prev_holdoff = balance_retry_holdoff;
        if balance_retry_holdoff > 0 {
            balance_retry_holdoff = balance_retry_holdoff.saturating_sub(1);
        }
        // 当抑制计时刚好归零时，立刻执行一次快速评估，避免再等到下一周期
        if prev_holdoff > 0 && balance_retry_holdoff == 0 {
            if adapter_present && !TEST_FORCE_BQ_FETS_OFF {
                debug!("bal:holdoff");
                execute_smart_battery_balancing(
                    &mut bq,
                    &latest_core_measurements,
                    &mut active_balancing_cell,
                )
                .await;
            }
        }

        // Update edge tracking
        prev_adapter_present = adapter_present;
        prev_charger_confirmed = charger_confirmed;
    }
}
