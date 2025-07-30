use bq769x0_async_rs::RegisterAccess;
use bq769x0_async_rs::registers::{Register, SysCtrl2Flags, SysStatFlags};
use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::{gpio::Input, i2c::I2c, peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq769x0_async_rs::ProtectionConfig;
use bq769x0_async_rs::{
    BatteryConfig, Bq769x0, data_types::NtcParameters, errors::Error as BQ769x0Error,
};

// Import necessary data types
use crate::shared::{Bq76920AlertsPublisher, Bq76920MeasurementsPublisher};

// Smart cell balancing logic based on charging status and voltage thresholds
async fn execute_smart_battery_balancing<'a>(
    bq: &'a mut Bq769x0<
        I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, peripherals::I2C0, embassy_rp::i2c::Async>,
        >,
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
        info!(
            "  Pack voltage diff threshold: {} mV",
            PACK_VOLTAGE_DIFF_THRESHOLD_MV
        );

        // Display all cell voltages and find min/max
        let mut valid_cells = 0;
        let mut min_voltage = i32::MAX;
        let mut max_voltage = 0i32;

        for i in 0..NUM_CELLS {
            let voltage = measurements.cell_voltages.voltages[i];
            if voltage > 0 {
                // Only include valid readings
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
            info!(
                "  Pack voltage difference: {} mV (max: {} mV, min: {} mV)",
                pack_voltage_diff, max_voltage, min_voltage
            );

            // Check if balancing is allowed
            let balancing_allowed = pack_voltage_diff > PACK_VOLTAGE_DIFF_THRESHOLD_MV
                && (is_charging || min_voltage > MIN_VOLTAGE_THRESHOLD_MV);

            info!(
                "  Balancing allowed: {} (pack_diff > {}mV: {}, charging_or_min_ok: {})",
                balancing_allowed,
                PACK_VOLTAGE_DIFF_THRESHOLD_MV,
                pack_voltage_diff > PACK_VOLTAGE_DIFF_THRESHOLD_MV,
                is_charging || min_voltage > MIN_VOLTAGE_THRESHOLD_MV
            );

            if balancing_allowed {
                // Find the highest voltage cells for balancing
                let mut cells_need_balancing: [bool; NUM_CELLS] = [false; NUM_CELLS];
                let mut balance_candidate_count = 0;

                // Create a list of valid cells with their voltages and indices
                let mut valid_cells_list: [(usize, i32); NUM_CELLS] = [(0, 0); NUM_CELLS];
                let mut valid_count = 0;

                for i in 0..NUM_CELLS {
                    let voltage = measurements.cell_voltages.voltages[i];
                    if voltage > 0 {
                        // Only include valid readings
                        valid_cells_list[valid_count] = (i, voltage);
                        valid_count += 1;
                    }
                }

                if valid_count >= 2 {
                    // Sort cells by voltage (highest first) - simple bubble sort for small arrays
                    for i in 0..valid_count {
                        for j in 0..valid_count - 1 - i {
                            if valid_cells_list[j].1 < valid_cells_list[j + 1].1 {
                                valid_cells_list.swap(j, j + 1);
                            }
                        }
                    }

                    info!("  Cells sorted by voltage (highest first):");
                    for (idx, &(cell_idx, voltage)) in
                        valid_cells_list.iter().enumerate().take(valid_count)
                    {
                        info!("    {}. Cell {}: {} mV", idx + 1, cell_idx + 1, voltage);
                    }

                    // Check if the highest voltage cells need balancing
                    // Balance cells that are significantly higher than the minimum
                    let min_cell_voltage = valid_cells_list[valid_count - 1].1; // Lowest voltage (last after sorting)

                    for &(cell_idx, voltage) in valid_cells_list.iter().take(valid_count) {
                        let voltage_diff = voltage - min_cell_voltage;

                        if voltage_diff >= MIN_BALANCE_DIFF_MV && balance_candidate_count < 2 {
                            cells_need_balancing[cell_idx] = true;
                            balance_candidate_count += 1;
                            info!(
                                "  Cell {} ({} mV) needs balancing (diff: {} mV > {} mV)",
                                cell_idx + 1,
                                voltage,
                                voltage_diff,
                                MIN_BALANCE_DIFF_MV
                            );
                        } else if voltage_diff < MIN_BALANCE_DIFF_MV {
                            info!(
                                "  Cell {} ({} mV) skipped balancing (diff: {} mV < {} mV)",
                                cell_idx + 1,
                                voltage,
                                voltage_diff,
                                MIN_BALANCE_DIFF_MV
                            );
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

                    for (i, &needs_balancing) in
                        cells_need_balancing.iter().enumerate().take(NUM_CELLS)
                    {
                        if needs_balancing {
                            balancing_mask |= 1 << i;
                            info!(
                                "  Balancing Cell {}: {} mV",
                                i + 1,
                                measurements.cell_voltages.voltages[i]
                            );
                        }
                    }

                    info!(
                        "Enabling cell balancing: mask = 0b{:05b} ({} cells)",
                        balancing_mask, balance_candidate_count
                    );
                    if let Err(_e) = bq.set_cell_balancing(balancing_mask).await {
                        error!("Failed to enable cell balancing");
                    } else {
                        info!("Cell balancing enabled successfully.");
                    }
                } else {
                    info!("No cells need balancing - disabling balancing");
                    // Disable cell balancing
                    if let Err(_e) = bq.set_cell_balancing(0).await {
                        error!("Failed to disable cell balancing");
                    } else {
                        info!("Cell balancing disabled - no imbalance.");
                    }
                }
            } else {
                info!("Balancing not allowed - disabling balancing");
                info!(
                    "  Pack voltage diff {} mV <= {} mV OR (not charging AND min voltage {} mV <= {} mV)",
                    pack_voltage_diff,
                    PACK_VOLTAGE_DIFF_THRESHOLD_MV,
                    min_voltage,
                    MIN_VOLTAGE_THRESHOLD_MV
                );

                // Disable cell balancing
                if let Err(_e) = bq.set_cell_balancing(0).await {
                    error!("Failed to disable cell balancing");
                } else {
                    info!("Cell balancing disabled - conditions not met.");
                }
            }
        } else {
            info!("Insufficient valid cell readings for balancing");
        }
    } else {
        info!("No measurements available for balancing");
    }
}

/// BQ76920 battery management task for RP2040
///
/// This task manages the BQ76920 battery management IC, including:
/// - Battery monitoring (voltage, current, temperature)
/// - Safety protection (overvoltage, undervoltage, overcurrent)
/// - Cell balancing
/// - MOS FET control for charging and discharging
///
/// # Arguments
///
/// * `i2c_bus`: A shared I2C bus device for communication with the BQ76920.
/// * `address`: The I2C address of the BQ76920 chip.
/// * `sense_resistor_m_ohm`: Sense resistor value in mOhms
/// * `ntc_params`: NTC parameters for temperature sensing
/// * `discharge_control`: GPIO pin for discharge control (GP3)
/// * `charge_control`: GPIO pin for charge control (GP4)
/// * `bq76920_alerts_publisher`: Publisher for sending BQ76920 alert data.
/// * `bq76920_measurements_publisher`: Publisher for sending BQ76920 measurement data.
#[embassy_executor::task]
#[allow(clippy::too_many_arguments)]
pub async fn bq76920_task(
    i2c_bus: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, peripherals::I2C0, embassy_rp::i2c::Async>,
    >,
    address: u8,
    sense_resistor_m_ohm: u32,
    ntc_params: Option<NtcParameters>,
    discharge_control: Input<'static>,
    charge_control: Input<'static>,
    bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>,
) {
    info!("BQ76920 task started");

    // Initialize the BQ769x0 driver instance with CRC enabled and for 5 cells.
    let mut bq: Bq769x0<
        I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, peripherals::I2C0, embassy_rp::i2c::Async>,
        >,
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

    // Attempt to apply the configuration and verify that key safety registers
    // have been written correctly by reading them back.
    match bq.try_apply_config(&battery_config).await {
        Ok(_) => {
            info!("BQ76920 configuration applied successfully");
            // If configuration is verified, proceed to enable the Discharge FET.
            // Charge FET will be controlled by charge_control pin in the main loop.
            let _ = bq.enable_discharging().await;
            info!("BQ76920 discharge FET enabled");
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
        }
        Err(_e) => {
            // Handles other errors from try_apply_config, such as I2C communication errors.
            error!("CRITICAL: Failed to apply BQ76920 configuration due to other error");
            error!("FETs will NOT be enabled. System may be unsafe.");
        }
    }

    // Main loop for continuous data acquisition and publishing.
    let mut balance_timer_counter: u32 = 0; // Counter for battery balancing frequency

    loop {
        info!("--- Reading BQ76920 Data ---");

        // Read ADC calibration values (not used in current logging but kept for potential future use)
        let (_adc_gain_uv_per_lsb, _adc_offset_mv) = match bq.read_adc_calibration().await {
            Ok(cal) => cal,
            Err(_e) => {
                error!("Failed to read ADC calibration");
                // Use default calibration values if reading fails
                (365, 0) // Default values from datasheet
            }
        };

        // Read and display cell balancing status
        let cellbal1_register = bq.read_register(Register::CELLBAL1).await.unwrap_or(0);
        info!("Cell Balancing Status:");
        info!(
            "  CELLBAL1 register: 0b{:08b} (0x{:02X})",
            cellbal1_register, cellbal1_register
        );

        // Display which cells are enabled for balancing
        let mut balancing_cells = [0u8; 5];
        let mut balancing_count = 0;
        for i in 0..5 {
            if (cellbal1_register & (1 << i)) != 0 {
                balancing_cells[balancing_count] = (i + 1) as u8;
                balancing_count += 1;
            }
        }

        if balancing_count == 0 {
            info!("  No cells are currently balancing");
        } else {
            info!(
                "  Cells currently balancing: {:?}",
                &balancing_cells[..balancing_count]
            );
        }

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
                info!(
                    "  TS1: {} ({}°C)",
                    core_meas.temperatures.ts1,
                    core_meas.temperatures.ts1 as f32 / 100.0
                );
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
                info!(
                    "  CC Ready: {}",
                    core_meas.system_status.0.contains(SysStatFlags::CC_READY)
                );
                info!(
                    "  Overtemperature: {}",
                    core_meas.system_status.0.contains(SysStatFlags::OVRD_ALERT)
                );

                let uv_fault = core_meas.system_status.0.contains(SysStatFlags::UV);
                info!("  Undervoltage (UV): {}", uv_fault);
                info!(
                    "  Overvoltage (OV): {}",
                    core_meas.system_status.0.contains(SysStatFlags::OV)
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

                // GPIO discharge control: Check if discharge control pin is connected to GND (low level)
                let gpio_enable_discharge = discharge_control.is_low();

                // Combined discharge control logic: UV fault management + GPIO control
                let should_enable_discharge = !uv_fault && gpio_enable_discharge;
                let is_discharge_currently_on =
                    core_meas.mos_status.0.contains(SysCtrl2Flags::DSG_ON);

                if should_enable_discharge && !is_discharge_currently_on {
                    info!("Enabling discharge FET");
                    let _ = bq.enable_discharging().await;
                } else if !should_enable_discharge && is_discharge_currently_on {
                    info!("Disabling discharge FET");
                    let _ = bq.disable_discharging().await;
                }

                // GPIO charge control: Check if charge control pin is connected to GND (low level)
                let gpio_allow_charging = charge_control.is_low();

                // Combined charge control logic: OV fault management + GPIO control
                let ov_fault = core_meas.system_status.0.contains(SysStatFlags::OV);
                let should_enable_charging = !ov_fault && gpio_allow_charging;
                let is_charging_currently_on =
                    core_meas.mos_status.0.contains(SysCtrl2Flags::CHG_ON);

                if should_enable_charging && !is_charging_currently_on {
                    info!("Enabling charge FET");
                    let _ = bq.enable_charging().await;
                } else if !should_enable_charging && is_charging_currently_on {
                    info!("Disabling charge FET");
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
                        error!("Failed to clear BQ76920 status flags");
                    } else {
                        info!("Cleared BQ76920 status flags: {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(_e) => {
                error!("Failed to read BQ76920 measurements");
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
        if balance_timer_counter == 0 || balance_timer_counter >= 3600 {
            // 3600 seconds = 1 hour
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
