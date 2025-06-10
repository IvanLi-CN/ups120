use bq769x0_async_rs::registers::CellBal1Flags;
use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// Removed WaitResult import as it's no longer needed in this task

// use bq769x0_async_rs::registers::*; // Removed unused import
// use bq769x0_async_rs::units::ElectricalResistance; // Removed as uom is no longer used by the lib
use bq769x0_async_rs::ProtectionConfig;
use bq769x0_async_rs::{
    BatteryConfig, Bq769x0, data_types::NtcParameters, errors::Error as BQ769x0Error,
}; // Import Error, removed RegisterAccess, Added NtcParameters // Added to resolve E0422

// Import necessary data types
use crate::shared::{
    Bq76920AlertsPublisher,
    Bq76920MeasurementsPublisher, // Added Bq76920MeasurementsPublisher
};

// New helper function for battery balancing logic
async fn execute_battery_balancing<'a>(
    bq: &'a mut Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
        bq769x0_async_rs::Enabled,
        5,
    >,
    latest_core_measurements: &'a Option<bq769x0_async_rs::data_types::Bq76920Measurements<5>>,
) {
    if let Some(measurements) = latest_core_measurements {
        let mut balance_flags = CellBal1Flags::empty();
        let mut total_voltage = 0;
        let mut cell_count = 0;

        for voltage in measurements.cell_voltages.voltages.iter() {
            total_voltage += *voltage;
            cell_count += 1;
        }

        if cell_count > 0 {
            let average_voltage = total_voltage / cell_count;
            let balance_threshold_mv = 50; // Example threshold: 50mV

            for (i, voltage) in measurements.cell_voltages.voltages.iter().enumerate() {
                if *voltage > average_voltage + balance_threshold_mv {
                    // Set the corresponding balance flag for cell i+1
                    match i {
                        0 => balance_flags |= CellBal1Flags::BAL1,
                        1 => balance_flags |= CellBal1Flags::BAL2,
                        2 => balance_flags |= CellBal1Flags::BAL3,
                        3 => balance_flags |= CellBal1Flags::BAL4,
                        4 => balance_flags |= CellBal1Flags::BAL5,
                        _ => {} // Should not happen for BQ76920 with 5 cells
                    }
                }
            }

            // Write the calculated balance flags to the CELLBAL1 register
            if !balance_flags.is_empty() {
                info!("Attempting to set BQ76920 cell balance flags: {:#010b}", balance_flags.bits());
                if let Err(e) = bq.set_cell_balancing(balance_flags.bits() as u16).await {
                    error!("Failed to set BQ76920 cell balance flags using set_cell_balancing: {:?}", e);
                } else {
                    info!("BQ76920 cell balance flags set.");
                }
            } else {
                 // If no cells need balancing, ensure balance flags are cleared
                 if let Err(e) = bq.set_cell_balancing(CellBal1Flags::empty().bits() as u16).await {
                    error!("Failed to clear BQ76920 cell balance flags using set_cell_balancing: {:?}", e);
                }
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

        // Read all measurements from BQ76920. These are now in physical units.
        match bq.read_all_measurements().await {
            Ok(core_meas) => {
                latest_core_measurements = Some(core_meas);

                // Log all BQ76920 measurements in a single line
                info!(
                    "BQ76920: Cells={:?}mV, Total={}mV, Current={}mA",
                    core_meas.cell_voltages.voltages,
                    core_meas.total_voltage_mv,
                    core_meas.current_ma
                );

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
            execute_battery_balancing(&mut bq, &latest_core_measurements).await;
            balance_timer_counter = 0; // Reset counter after execution
        }
        // --- End Battery Balancing Logic ---

        // Wait for a defined interval before the next cycle of readings.
        Timer::after(Duration::from_secs(1)).await;
        balance_timer_counter += 1;
    }
}
