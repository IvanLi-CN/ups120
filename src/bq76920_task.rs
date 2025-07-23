use bq769x0_async_rs::registers::{Register, SysCtrl1Flags, SysCtrl2Flags, SysStatFlags};
use bq769x0_async_rs::RegisterAccess;
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
use crate::shared::{
    Bq76920AlertsPublisher,
    Bq76920MeasurementsPublisher,
};

// Smart cell balancing logic based on charging status and voltage thresholds
async fn execute_smart_battery_balancing<'a>(
    bq: &'a mut Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
        bq769x0_async_rs::Enabled,
        5,
    >,
    latest_core_measurements: &'a Option<bq769x0_async_rs::data_types::Bq76920Measurements<5>>,
) {
    const NUM_CELLS: usize = 5;
    const MIN_VOLTAGE_THRESHOLD_MV: i32 = 3300; // Minimum voltage threshold for balancing
    const PACK_VOLTAGE_DIFF_THRESHOLD_MV: i32 = 10; // Pack voltage difference threshold
    const MIN_BALANCE_DIFF_MV: i32 = 2; // Minimum voltage difference to trigger balancing

    if let Some(measurements) = latest_core_measurements {
        // Determine if charging: CHG_ON is enabled AND current is positive (flowing into battery)
        let chg_on = measurements.mos_status.0.contains(SysCtrl2Flags::CHG_ON);
        let is_charging = chg_on && measurements.current_ma > 10; // 10mA threshold to avoid noise

        info!("Battery Status:");
        info!("  Charging: {}", is_charging);
        info!("  Min voltage threshold: {} mV", MIN_VOLTAGE_THRESHOLD_MV);
        info!("  Pack voltage diff threshold: {} mV", PACK_VOLTAGE_DIFF_THRESHOLD_MV);

        // Display all cell voltages and find min/max
        let mut valid_cells = 0;
        let mut min_voltage = i32::MAX;
        let mut max_voltage = 0i32;

        for i in 0..NUM_CELLS {
            let voltage = measurements.cell_voltages.voltages[i];
            if voltage > 0 { // Only include valid readings
                info!("  Cell {}: {} mV", i + 1, voltage);
                valid_cells += 1;
                if voltage < min_voltage {
                    min_voltage = voltage;
                }
                if voltage > max_voltage {
                    max_voltage = voltage;
                }
            }
        }

        if valid_cells >= 2 {
            let pack_voltage_diff = max_voltage - min_voltage;
            info!("  Pack voltage difference: {} mV (max: {} mV, min: {} mV)",
                 pack_voltage_diff, max_voltage, min_voltage);

            // Check if balancing is allowed
            let balancing_allowed = pack_voltage_diff > PACK_VOLTAGE_DIFF_THRESHOLD_MV &&
                                  (is_charging || min_voltage > MIN_VOLTAGE_THRESHOLD_MV);

            info!("  Balancing allowed: {} (pack_diff > {}mV: {}, charging_or_min_ok: {})",
                 balancing_allowed,
                 PACK_VOLTAGE_DIFF_THRESHOLD_MV,
                 pack_voltage_diff > PACK_VOLTAGE_DIFF_THRESHOLD_MV,
                 is_charging || min_voltage > MIN_VOLTAGE_THRESHOLD_MV);

            if balancing_allowed {
                // Find the highest voltage cells for balancing
                let mut cells_need_balancing: [bool; NUM_CELLS] = [false; NUM_CELLS];
                let mut balance_candidate_count = 0;

                // Create a list of valid cells with their voltages and indices
                let mut valid_cells_list: [(usize, i32); NUM_CELLS] = [(0, 0); NUM_CELLS];
                let mut valid_count = 0;

                for i in 0..NUM_CELLS {
                    let voltage = measurements.cell_voltages.voltages[i];
                    if voltage > 0 { // Only include valid readings
                        valid_cells_list[valid_count] = (i, voltage);
                        valid_count += 1;
                    }
                }

                if valid_count >= 2 {
                    // Sort cells by voltage (highest first) - simple bubble sort for small arrays
                    for i in 0..valid_count {
                        for j in 0..valid_count - 1 - i {
                            if valid_cells_list[j].1 < valid_cells_list[j + 1].1 {
                                let temp = valid_cells_list[j];
                                valid_cells_list[j] = valid_cells_list[j + 1];
                                valid_cells_list[j + 1] = temp;
                            }
                        }
                    }

                    info!("  Cells sorted by voltage (highest first):");
                    for idx in 0..valid_count {
                        let (cell_idx, voltage) = valid_cells_list[idx];
                        info!("    {}. Cell {}: {} mV", idx + 1, cell_idx + 1, voltage);
                    }

                    // Check if the highest voltage cells need balancing
                    // Balance cells that are significantly higher than the minimum
                    let min_cell_voltage = valid_cells_list[valid_count - 1].1; // Lowest voltage (last after sorting)

                    for idx in 0..valid_count {
                        let (cell_idx, voltage) = valid_cells_list[idx];
                        let voltage_diff = voltage - min_cell_voltage;

                        if voltage_diff >= MIN_BALANCE_DIFF_MV && balance_candidate_count < 2 {
                            cells_need_balancing[cell_idx] = true;
                            balance_candidate_count += 1;
                            info!("  Cell {} ({} mV) needs balancing (diff: {} mV > {} mV)",
                                 cell_idx + 1, voltage, voltage_diff, MIN_BALANCE_DIFF_MV);
                        } else if voltage_diff < MIN_BALANCE_DIFF_MV {
                            info!("  Cell {} ({} mV) skipped balancing (diff: {} mV < {} mV)",
                                 cell_idx + 1, voltage, voltage_diff, MIN_BALANCE_DIFF_MV);
                        }

                        if balance_candidate_count >= 2 {
                            info!("  Reached maximum 2 cells for balancing");
                            break;
                        }
                    }
                }

                info!("  Cells needing balancing: {}", balance_candidate_count);

                if balance_candidate_count > 0 {
                    let mut balancing_mask: u16 = 0;

                    for i in 0..NUM_CELLS {
                        if cells_need_balancing[i] {
                            balancing_mask |= 1 << i;
                            info!("  Balancing Cell {}: {} mV", i + 1, measurements.cell_voltages.voltages[i]);
                        }
                    }

                    info!("Enabling cell balancing: mask = 0b{:05b} ({} cells)", balancing_mask, balance_candidate_count);
                    if let Err(e) = bq.set_cell_balancing(balancing_mask).await {
                        error!("Failed to enable cell balancing: {:?}", e);
                    } else {
                        info!("Cell balancing enabled successfully.");
                    }
                } else {
                    info!("No cells need balancing - disabling balancing");
                    // Disable cell balancing
                    if let Err(e) = bq.set_cell_balancing(0).await {
                        error!("Failed to disable cell balancing: {:?}", e);
                    } else {
                        info!("Cell balancing disabled - no imbalance.");
                    }
                }
            } else {
                info!("Balancing not allowed - disabling balancing");
                info!("  Pack voltage diff {} mV <= {} mV OR (not charging AND min voltage {} mV <= {} mV)",
                     pack_voltage_diff, PACK_VOLTAGE_DIFF_THRESHOLD_MV, min_voltage, MIN_VOLTAGE_THRESHOLD_MV);

                // Disable cell balancing
                if let Err(e) = bq.set_cell_balancing(0).await {
                    error!("Failed to disable cell balancing: {:?}", e);
                } else {
                    info!("Cell balancing disabled - conditions not met.");
                }
            }
        } else {
            error!("Insufficient valid cell voltage readings for balancing (need at least 2 cells)");
            // Disable balancing if insufficient data
            if let Err(e) = bq.set_cell_balancing(0).await {
                error!("Failed to disable cell balancing: {:?}", e);
            }
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
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    sense_resistor_m_ohm: u32, // Added: Sense resistor value in mOhms
    ntc_params: Option<NtcParameters>, // Added: NTC parameters
    bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>,
) {
    info!("BQ76920 task started.");

    // Initialize the BQ769x0 driver instance with CRC enabled and for 5 cells.
    // sense_resistor_m_ohm and ntc_params are now passed as arguments to this task.
    let mut bq: Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
        bq769x0_async_rs::Enabled,
        5,
    > = Bq769x0::new(i2c_bus, address, sense_resistor_m_ohm, ntc_params);

    // Variables to store the latest readings from the sub-module, which are now in physical units.
    #[allow(unused_assignments)]
    let mut latest_core_measurements: Option<bq769x0_async_rs::data_types::Bq76920Measurements<5>> =
        None;

    // --- BQ76920 Initialization Sequence ---

    // Note: Waking the BQ76920 from SHIP mode (if it was in that mode)
    // is typically handled by external hardware, e.g., by pulling the TS1 pin high.
    // This task assumes the chip is already in NORMAL mode or has been woken up by such means.

    // Define the battery configuration.
    // Start with default values and then override specific parameters.
    // Define the battery configuration using struct update syntax.
    // `sense_resistor_uohms` is defined earlier in the function.
    let battery_config = BatteryConfig {
        overvoltage_trip: 3600u32,  // Set to 3.6V
        undervoltage_trip: 2500u32, // Set to 2.5V
        protection_config: ProtectionConfig {
            ocd_limit: 10_000i32,                         // Set to 10A (10_000 mA)
            ..BatteryConfig::default().protection_config  // Inherit other protection_config fields
        },
        rsense: sense_resistor_m_ohm, // Use mOhms directly as per BatteryConfig field
        ..Default::default()          // Inherit other BatteryConfig fields
    };

    let mut fets_enabled_after_config = false;

    // Attempt to apply the configuration and, critically, verify that key safety registers
    // have been written correctly by reading them back.
    match bq.try_apply_config(&battery_config).await {
        Ok(_) => {
            info!("BQ76920 configuration applied and verified successfully.");

            // If configuration is verified, proceed to enable the Charge and Discharge FETs.
            // This allows the BQ76920 to control the battery pack's connection to charger/load.
            info!("Attempting to enable BQ76920 Charge FET (CHG_ON)...");
            if let Err(e) = bq.enable_charging().await {
                error!("Failed to enable BQ76920 Charge FET: {:?}", e);
            } else {
                info!("BQ76920 Charge FET (CHG_ON) enabled command sent.");
            }

            info!("Attempting to enable BQ76920 Discharge FET (DSG_ON)...");
            if let Err(e) = bq.enable_discharging().await {
                error!("Failed to enable BQ76920 Discharge FET: {:?}", e);
            } else {
                info!("BQ76920 Discharge FET (DSG_ON) enabled command sent.");
            }
            fets_enabled_after_config = true; // Mark that FETs were attempted to be enabled.
        }
        Err(BQ769x0Error::ConfigVerificationFailed {
            register,
            expected,
            actual,
        }) => {
            // This is a CRITICAL error. Configuration did not write correctly.
            // FETs will NOT be enabled to prevent potentially unsafe operation
            // with incorrect protection settings.
            error!("CRITICAL: BQ76920 CONFIGURATION VERIFICATION FAILED!");
            error!("  Register: {:?}", register);
            error!("  Expected: {:#04x}", expected);
            error!("  Actual:   {:#04x}", actual);
            error!(
                "FETs will NOT be enabled due to this configuration error. System may be unsafe."
            );
            // Depending on system requirements, this might warrant a panic or a safe shutdown procedure.
        }
        Err(e) => {
            // Handles other errors from try_apply_config, such as I2C communication errors.
            // Also a CRITICAL failure scenario.
            error!(
                "CRITICAL: Failed to apply BQ76920 configuration due to other error: {:?}",
                e
            );
            error!("FETs will NOT be enabled. System may be unsafe.");
        }
    }

    if fets_enabled_after_config {
        info!("BQ76920 initialization and FET enable sequence complete.");
    } else {
        warn!(
            "BQ76920 initialization complete, but FETs were NOT enabled due to prior configuration issues."
        );
    }

    // Runtime config (Bq76920RuntimeConfig) is no longer published from here,
    // as NTC parameters and sense resistor are now part of Bq769x0 driver initialization.

    // Main loop for continuous data acquisition and publishing.
    let mut balance_timer_counter: u32 = 0; // Counter for battery balancing frequency

    loop {
        // This task focuses on reading data from the BQ76920 itself.
        // Communication with other chips (like BQ25730 charger) is handled in their respective tasks.

        // Note: The CC_EN (Coulomb Counter Enable) flag in SYS_CTRL2 is set by default
        // in `BatteryConfig::default()` and verified by `try_apply_config`.
        // Therefore, an explicit check and write for CC_EN in this loop is no longer necessary.

        info!("--- Reading BQ76920 Data ---");

        // Check system control register status (for debugging)
        info!("Checking system control register status...");
        let sys_ctrl1_val = bq.read_register(Register::SysCtrl1).await.unwrap_or(0);
        let sys_ctrl1_flags = SysCtrl1Flags::from_bits_truncate(sys_ctrl1_val);
        let sys_ctrl2_val = bq.read_register(Register::SysCtrl2).await.unwrap_or(0);
        let sys_ctrl2_flags = SysCtrl2Flags::from_bits_truncate(sys_ctrl2_val);

        info!(
            "SYS_CTRL1 = 0x{:02X}, ADC_EN = {}",
            sys_ctrl1_val,
            sys_ctrl1_flags.contains(SysCtrl1Flags::ADC_EN)
        );
        info!(
            "SYS_CTRL2 = 0x{:02X}, CC_EN = {}",
            sys_ctrl2_val,
            sys_ctrl2_flags.contains(SysCtrl2Flags::CC_EN)
        );

        // Read ADC calibration values
        let (adc_gain_uv_per_lsb, adc_offset_mv) = match bq.read_adc_calibration().await {
            Ok(cal) => cal,
            Err(e) => {
                error!("Failed to read ADC calibration: {:?}", e);
                // Use default calibration values if reading fails
                (365, 0) // Default values from datasheet
            }
        };
        info!(
            "ADC Calibration: Gain={} uV/LSB, Offset={} mV",
            adc_gain_uv_per_lsb, adc_offset_mv
        );

        // Debug UV_TRIP register and calculation
        let uv_trip_register = bq.read_register(Register::UvTrip).await.unwrap_or(0);
        info!("UV_TRIP register value: 0x{:02X}", uv_trip_register);

        // Calculate what voltage this UV_TRIP register represents
        // UV threshold format: 01-XXXXXXXX-0000 (14-bit)
        let uv_trip_14bit = (0b01 << 12) | ((uv_trip_register as u16) << 4) | 0b0000;
        let uv_trip_voltage_mv =
            (uv_trip_14bit as i32 * adc_gain_uv_per_lsb as i32) / 1000 + adc_offset_mv as i32;
        info!(
            "Calculated UV trip voltage: {} mV (from 14-bit value: 0x{:04X})",
            uv_trip_voltage_mv, uv_trip_14bit
        );

        // Also check OV_TRIP for comparison
        let ov_trip_register = bq.read_register(Register::OvTrip).await.unwrap_or(0);
        info!("OV_TRIP register value: 0x{:02X}", ov_trip_register);
        let ov_trip_14bit = (0b10 << 12) | ((ov_trip_register as u16) << 4) | 0b1000;
        let ov_trip_voltage_mv =
            (ov_trip_14bit as i32 * adc_gain_uv_per_lsb as i32) / 1000 + adc_offset_mv as i32;
        info!(
            "Calculated OV trip voltage: {} mV (from 14-bit value: 0x{:04X})",
            ov_trip_voltage_mv, ov_trip_14bit
        );

        // Read all measurements from BQ76920. These are now in physical units.
        match bq.read_all_measurements().await {
            Ok(core_meas) => {
                latest_core_measurements = Some(core_meas);

                // Log detailed BQ76920 measurements
                info!("Cell Voltages:");
                for i in 0..5 {
                    let voltage_mv = core_meas.cell_voltages.voltages[i];
                    info!("  Cell {}: {} mV", i + 1, voltage_mv);
                }

                info!("Pack Voltage: {} mV", core_meas.total_voltage_mv);
                info!("Current: {} mA", core_meas.current_ma);

                // Log temperatures
                info!("Temperatures (0.01°C):");
                info!("  TS1: {} ({}°C)", core_meas.temperatures.ts1, core_meas.temperatures.ts1 as f32 / 100.0);
                if let Some(ts2) = core_meas.temperatures.ts2 {
                    info!("  TS2: {} ({}°C)", ts2, ts2 as f32 / 100.0);
                }
                if let Some(ts3) = core_meas.temperatures.ts3 {
                    info!("  TS3: {} ({}°C)", ts3, ts3 as f32 / 100.0);
                }

                // Log detailed system status
                info!(
                    "System Status (SYS_STAT register: 0x{:02X}):",
                    core_meas.system_status.0.bits()
                );
                info!("  CC Ready: {}", core_meas.system_status.0.contains(SysStatFlags::CC_READY));
                info!(
                    "  Overtemperature: {}",
                    core_meas.system_status.0.contains(SysStatFlags::OVRD_ALERT)
                );

                let uv_fault = core_meas.system_status.0.contains(SysStatFlags::UV);
                info!(
                    "  Undervoltage (UV): {} - Expected threshold: {} mV",
                    uv_fault, uv_trip_voltage_mv
                );
                info!(
                    "  Overvoltage (OV): {} - Expected threshold: {} mV",
                    core_meas.system_status.0.contains(SysStatFlags::OV),
                    ov_trip_voltage_mv
                );
                info!(
                    "  Short Circuit Discharge (SCD): {}",
                    core_meas.system_status.0.contains(SysStatFlags::SCD)
                );
                info!(
                    "  Overcurrent Discharge (OCD): {}",
                    core_meas.system_status.0.contains(SysStatFlags::OCD)
                );

                // Log MOS status
                let chg_on = core_meas.mos_status.0.contains(SysCtrl2Flags::CHG_ON);
                info!("MOS Status:");
                info!("  Charge MOSFET (CHG_ON): {}", chg_on);
                info!(
                    "  Discharge MOSFET (DSG_ON): {}",
                    core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON)
                );
                info!(
                    "  Coulomb Counter (CC_EN): {}",
                    core_meas.mos_status.0.contains(SysCtrl2Flags::CC_EN)
                );
                info!(
                    "  CC One-Shot (CC_ONESHOT): {}",
                    core_meas.mos_status.0.contains(SysCtrl2Flags::CC_ONESHOT)
                );
                info!(
                    "  Delay Disable (DELAY_DIS): {}",
                    core_meas.mos_status.0.contains(SysCtrl2Flags::DELAY_DIS)
                );

                // UV fault management: Enable discharge MOS when UV fault is false (cleared)
                if !uv_fault {
                    // UV fault is not present, ensure discharge MOS is enabled
                    if !core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON) {
                        info!("UV fault cleared, enabling discharge MOSFET...");
                        if let Err(e) = bq.enable_discharging().await {
                            error!("Failed to enable discharging after UV clear: {:?}", e);
                        } else {
                            info!("Discharge MOSFET enabled after UV fault cleared.");
                        }
                    }
                } else {
                    // UV fault is present, discharge MOS should be disabled by hardware
                    info!(
                        "UV fault detected - discharge MOSFET should be disabled by hardware protection"
                    );
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
                        error!("Failed to clear BQ76920 status flags: {:?}", e_clear);
                    } else {
                        info!("Cleared BQ76920 status flags: {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(e) => {
                error!("Failed to read BQ76920 measurements: {:?}", e);
                latest_core_measurements = None;
                // Optionally publish default/error state for alerts if needed
                let alerts = crate::data_types::Bq76920Alerts::default();
                bq76920_alerts_publisher.publish_immediate(alerts);
            }
        }

        // Construct the BQ76920 measurements payload for the main `AllMeasurements` publisher.
        // If read_all_measurements failed, use default values.
        let bq76920_measurements_payload_for_main_pub = crate::data_types::Bq76920Measurements {
            core_measurements: latest_core_measurements.unwrap_or_default(),
        };

        // Publish the collected BQ76920 measurements (which are now wrapped in the main project's type).
        bq76920_measurements_publisher.publish_immediate(bq76920_measurements_payload_for_main_pub);

        // --- Battery Balancing Logic (executed approximately once per hour) ---
        if balance_timer_counter == 0 || balance_timer_counter >= 3600 { // 3600 seconds = 1 hour
            info!("Executing hourly battery balancing logic.");
            execute_smart_battery_balancing(&mut bq, &latest_core_measurements).await;
            balance_timer_counter = 0; // Reset counter after execution
        }
        // --- End Battery Balancing Logic ---

        info!("----------------------------");

        // Wait for a defined interval before the next cycle of readings.
        Timer::after(Duration::from_secs(1)).await;
        balance_timer_counter += 1;
    }
}
