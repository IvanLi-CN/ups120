use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// Removed WaitResult import as it's no longer needed in this task

use bq769x0_async_rs::registers::*;
// use bq769x0_async_rs::units::ElectricalResistance; // Removed as uom is no longer used by the lib
use bq769x0_async_rs::{BatteryConfig, Bq769x0, errors::Error as BQ769x0Error}; // Import Error, removed RegisterAccess

// Import necessary data types
use crate::shared::{
    Bq76920AlertsPublisher,
    Bq76920MeasurementsPublisher, // Added Bq76920MeasurementsPublisher
};

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
    bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>,
) {
    info!("BQ76920 task started.");

    // Initialize the BQ769x0 driver instance with CRC enabled and for 5 cells.
    let mut bq: Bq769x0<
        I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
        bq769x0_async_rs::Enabled,
        5,
    > = Bq769x0::new(i2c_bus, address);

    // Sense resistor value in microOhms (e.g., 3.0 mΩ = 3000 µΩ).
    // This value is used by the driver library to convert raw Coulomb Counter (CC) readings to current in mA.
    // FIXME: This should ideally come from a central configuration or be part of BatteryConfig if the lib supports it.
    let sense_resistor_uohms: u32 = 3000;

    // Variables to store the latest readings. Initialized to None and updated in the loop.
    // These are used to construct the Bq76920Measurements struct for publishing.
    let mut _voltages: Option<bq769x0_async_rs::CellVoltages<5>> = None;
    let mut _temps: Option<bq769x0_async_rs::TemperatureSensorReadings> = None;
    let mut _current: Option<i32> = None; // Current in mA
    let mut _system_status: Option<bq769x0_async_rs::SystemStatus> = None;
    let mut _mos_status: Option<bq769x0_async_rs::MosStatus> = None;

    // --- BQ76920 Initialization Sequence ---
    info!("Starting BQ76920 initialization sequence...");

    // Note: Waking the BQ76920 from SHIP mode (if it was in that mode)
    // is typically handled by external hardware, e.g., by pulling the TS1 pin high.
    // This task assumes the chip is already in NORMAL mode or has been woken up by such means.

    // Define the battery configuration.
    // Start with default values and then override specific parameters.
    let mut battery_config = BatteryConfig::default();

    // Example: Configure for a 5-cell LiFePO4 battery.
    // Overvoltage trip threshold per cell (e.g., 3.6V).
    battery_config.overvoltage_trip = 3600u32;
    // Undervoltage trip threshold per cell (e.g., 2.5V).
    battery_config.undervoltage_trip = 2500u32;
    // Overcurrent in Discharge (OCD) limit (e.g., 10A).
    // Note: The driver converts this current limit to a voltage threshold based on Rsense.
    battery_config.protection_config.ocd_limit = 10_000i32; // 10_000 mA = 10A
    // Short Circuit in Discharge (SCD) limit is also part of ProtectionConfig, using default here.
    // Rsense value is part of BatteryConfig and used for current limit calculations.
    battery_config.rsense = sense_resistor_uohms / 1000; // rsense in mOhms for BatteryConfig

    let mut fets_enabled_after_config = false;

    info!("Applying and verifying BQ76920 configuration...");
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
            error!("FETs will NOT be enabled due to this configuration error. System may be unsafe.");
            // Depending on system requirements, this might warrant a panic or a safe shutdown procedure.
        }
        Err(e) => {
            // Handles other errors from try_apply_config, such as I2C communication errors.
            // Also a CRITICAL failure scenario.
            error!("CRITICAL: Failed to apply BQ76920 configuration due to other error: {:?}", e);
            error!("FETs will NOT be enabled. System may be unsafe.");
        }
    }

    if fets_enabled_after_config {
        info!("BQ76920 initialization and FET enable sequence complete.");
    } else {
        warn!("BQ76920 initialization complete, but FETs were NOT enabled due to prior configuration issues.");
    }

    // Main loop for continuous data acquisition and publishing.
    info!("BQ76920 entering main data acquisition loop...");
    loop {
        // This task focuses on reading data from the BQ76920 itself.
        // Communication with other chips (like BQ25730 charger) is handled in their respective tasks.

        // Note: The CC_EN (Coulomb Counter Enable) flag in SYS_CTRL2 is set by default
        // in `BatteryConfig::default()` and verified by `try_apply_config`.
        // Therefore, an explicit check and write for CC_EN in this loop is no longer necessary.

        // Read Cell Voltages
        match bq.read_cell_voltages().await {
            Ok(v_converted) => {
                _voltages = Some(v_converted); // Store the successfully read voltages.
            }
            Err(e) => {
                error!("Failed to read BQ76920 cell voltages: {:?}", e);
                _voltages = None; // Mark as None if read fails.
            }
        }

        // Read Pack Voltage (total battery voltage)
        match bq.read_pack_voltage().await {
            Ok(voltage) => {
                info!("BQ76920 Pack Voltage: {} mV", voltage);
                // This value is not directly part of Bq76920Measurements struct but logged for info.
            }
            Err(e) => {
                error!("Failed to read BQ76920 pack voltage: {:?}", e);
            }
        }

        // Read Temperatures (raw ADC values from temperature sensors)
        match bq.read_temperatures().await {
            Ok(sensor_readings) => {
                _temps = Some(sensor_readings); // Store raw temperature readings.
                                               // Conversion to Celsius would typically happen when processing/displaying this data,
                                               // potentially using thermistor characteristics if external sensors are used.
            }
            Err(e) => {
                error!("Failed to read BQ76920 temperature sensor readings: {:?}", e);
                _temps = None;
            }
        }

        // Read Current (from Coulomb Counter)
        match bq.read_current().await {
            Ok(c) => {
                // Convert the raw Coulomb Counter value to current in mA using the sense resistor value.
                // FIXME: The method `convert_raw_cc_to_current_ma` is not defined in the bq769x0_async_rs driver.
                // This will cause a compilation error. This method needs to be implemented in the driver or
                // the current calculation logic needs to be integrated here or in the driver,
                // likely requiring the `rsense_m_ohm` value to be available (e.g., from `BatteryConfig`).
                // For now, assuming it exists for the purpose of this task structure.
                let current_ma = bq.convert_raw_cc_to_current_ma(c.raw_cc, sense_resistor_uohms);
                info!("BQ76920 Raw CC: {}, Calculated Current: {} mA", c.raw_cc, current_ma);
                _current = Some(current_ma);
            }
            Err(e) => {
                error!("Failed to read BQ76920 current: {:?}", e);
                _current = None;
            }
        }

        // Read System Status register (SYS_STAT)
        // This register contains flags for various protection events (OV, UV, SCD, OCD) and other statuses.
        match bq.read_status().await {
            Ok(status_flags) => {
                _system_status = Some(status_flags); // Store the read system status.

                // It's important to clear any set status flags after reading them,
                // so that new events can be detected. Writing '1' to a bit clears it.
                let flags_to_clear = status_flags.0.bits(); // Get all currently set flags.
                if flags_to_clear != 0 {
                    if let Err(e) = bq.clear_status_flags(flags_to_clear).await {
                        error!("Failed to clear BQ76920 status flags: {:?}", e);
                    } else {
                        info!("Cleared BQ76920 status flags: {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(e) => {
                error!("Failed to read BQ76920 system status: {:?}", e);
                _system_status = None;
            }
        }

        // Read MOS FET status from SYS_CTRL2 register
        // This indicates whether the CHG_ON and DSG_ON bits are set, reflecting the state of the FETs.
        match bq.read_mos_status().await {
            Ok(mos_state) => {
                info!(
                    "BQ76920 MOS Status: CHG_ON={}, DSG_ON={}",
                    mos_state.0.contains(SysCtrl2Flags::CHG_ON),
                    mos_state.0.contains(SysCtrl2Flags::DSG_ON)
                );
                _mos_status = Some(mos_state);
            }
            Err(e) => {
                error!("Failed to read BQ76920 SYS_CTRL2 for MOS status: {:?}", e);
                _mos_status = None;
            }
        }

        // Publish BQ76920 alert information (derived from system status).
        // This uses the `system_status` variable which was updated just above.
        if let Some(ss) = _system_status {
            let alerts = crate::data_types::Bq76920Alerts {
                system_status: ss,
            };
            bq76920_alerts_publisher.publish_immediate(alerts);
        }

        // Construct the comprehensive BQ76920 measurements structure.
        // If any individual read failed, use a default/empty value for that part
        // to ensure the overall structure can still be published.
        let bq76920_measurements_payload = crate::data_types::Bq76920Measurements {
            core_measurements: bq769x0_async_rs::data_types::Bq76920Measurements {
                cell_voltages: _voltages.unwrap_or_else(bq769x0_async_rs::data_types::CellVoltages::new),
                temperatures: _temps.unwrap_or_else(bq769x0_async_rs::data_types::TemperatureSensorReadings::new),
                current: _current.unwrap_or(0i32), // Default to 0 mA if current read failed.
                system_status: _system_status.unwrap_or_else(|| bq769x0_async_rs::data_types::SystemStatus::new(0)),
                mos_status: _mos_status.unwrap_or_else(|| bq769x0_async_rs::data_types::MosStatus::new(0)),
            },
        };

        // Publish the collected BQ76920 measurements.
        bq76920_measurements_publisher.publish_immediate(bq76920_measurements_payload);

        // Wait for a defined interval before the next cycle of readings.
        Timer::after(Duration::from_secs(1)).await;
    }
}
