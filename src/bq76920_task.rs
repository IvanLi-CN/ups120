use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// Removed WaitResult import as it's no longer needed in this task

use bq769x0_async_rs::registers::*;
// use bq769x0_async_rs::units::ElectricalResistance; // Removed as uom is no longer used by the lib
use bq769x0_async_rs::{BatteryConfig, Bq769x0, RegisterAccess};

 // Import necessary data types
use crate::shared::{
    Bq76920AlertsPublisher, Bq76920MeasurementsPublisher, // Added Bq76920MeasurementsPublisher
};

#[embassy_executor::task]
pub async fn bq76920_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    bq76920_alerts_publisher: Bq76920AlertsPublisher<'static>,
    bq76920_measurements_publisher: Bq76920MeasurementsPublisher<'static, 5>, // Added publisher for BQ76920 measurements - Added generic parameter
    // Removed bq25730_measurements_subscriber and ina226_measurements_subscriber
) {
    info!("BQ76920 task started.");
    let mut bq: Bq769x0<_, bq769x0_async_rs::Enabled, 5> = { Bq769x0::new(i2c_bus, address) };
    // --- Main Loop for Data Acquisition ---
    // Assuming the library now takes the sense resistor value, e.g., as f32 in milliohms for calculations.
    // This might need adjustment based on the updated bq769x0-async-rs API.
    // If convert_raw_cc_to_current_ma is removed or changed, this will also need an update.
    // Assuming the library now takes the sense resistor value as u32 in microOhms.
    // This might need adjustment based on the updated bq769x0-async-rs API.
    let sense_resistor_uohms: u32 = 3000; // 3.0 milliOhms = 3000 microOhms

    // Declare variables to hold read data, initialized to None
    let mut voltages = None;
    let mut temps = None;
    let mut current = None;
    let mut system_status = None;
    let mut mos_status = None;

    // --- BQ76920 Initialization Sequence ---

    // Note: Waking from SHIP mode is typically handled by external hardware (TS1 pin).
    // Assuming the chip is already in NORMAL mode or has been woken up.

    // Define battery configuration
    let battery_config = BatteryConfig::default();

    info!("Applying battery configuration...");
    if let Err(e) = bq.set_config(&battery_config).await {
        error!("Failed to apply battery configuration: {:?}", e);
        // Depending on your error handling strategy, you might want to panic or return here
        // core::panic!("Failed to apply battery configuration: {:?}", e);
    }
    info!("Battery configuration applied successfully.");

    // Set CC_CFG register to 0x19 for optimal performance
    info!("Setting CC_CFG register to 0x19...");
    if let Err(e) = bq.write_register(Register::CcCfg, 0x19).await {
        error!("Failed to set CC_CFG: {:?}", e);
        // Depending on your error handling strategy, you might want to panic or return here
        // core::panic!("Failed to set CC_CFG: {:?}", e);
    }
    info!("CC_CFG set successfully.");

    // 4. Clear initial fault flags
    // Write 0xFF to SYS_STAT to clear all flags
    info!("Clearing initial status flags (writing 0xFF to SYS_STAT)...");
    if let Err(e) = bq.clear_status_flags(0xFF).await {
        error!("Failed to clear status flags: {:?}", e);
        // Depending on your error handling strategy, you might want to panic or return here
        // core::panic!("Failed to clear status flags: {:?}", e);
    }
    info!("Initial status flags cleared successfully.");

    info!("BQ76920 initialization complete.");

    // Removed variables for BQ25730 and INA226 latest measurements

    loop {
        // BQ76920 task only reads its own data

        info!("--- Reading BQ76920 Data ---");

        // Ensure CC_EN is enabled in SYS_CTRL2
        info!("Ensuring CC_EN is enabled in SYS_CTRL2...");
        let sys_ctrl2_val = bq.read_register(Register::SysCtrl2).await.unwrap_or(0);
        if !SysCtrl2Flags::from_bits_truncate(sys_ctrl2_val).contains(SysCtrl2Flags::CC_EN) {
            if let Err(e) = bq
                .write_register(
                    Register::SysCtrl2,
                    sys_ctrl2_val | SysCtrl2Flags::CC_EN.bits(),
                )
                .await
            {
                error!("Failed to enable CC_EN: {:?}", e);
            }
        }
        info!("CC_EN enable attempt complete.");

        // Read Cell Voltages
        let voltages_ref = &mut voltages;
        match bq.read_cell_voltages().await { // This now returns CellVoltages with [i32; N] (converted mV)
            Ok(v_converted) => {
                info!("Cell Voltages (mV):");
                for i in 0..5 {
                    info!(
                        "  Cell {}: {} mV",
                        i + 1,
                        v_converted.voltages[i] // This is already in mV
                    );
                }
                *voltages_ref = Some(v_converted); // Store the converted CellVoltages
            }
            Err(e) => {
                error!("Failed to read cell voltages: {:?}", e);
                *voltages_ref = None;
            }
        }

        // Read Pack Voltage
        match bq.read_pack_voltage().await {
            Ok(voltage) => {
                info!(
                    "Pack Voltage: {} mV",
                    voltage // Assuming this is already in mV (e.g., u32)
                );
            }
            Err(e) => {
                error!("Failed to read pack voltage: {:?}", e);
            }
        }

        // Read Temperatures
        let temps_ref = &mut temps;
        match bq.read_temperatures().await {
            Ok(sensor_readings) => {
                // Store the original sensor readings
                *temps_ref = Some(sensor_readings);

                // Convert sensor readings to temperature data for display
                match sensor_readings.into_temperature_data(None) {
                    // Assuming internal sensor, no NTC params
                    Ok(temp_data) => {
                        info!("Temperatures (Celsius):");
                        info!(
                            "  TS1: {} °C",
                            temp_data.ts1 as f32 / 100.0
                        );
                        if let Some(ts2_val) = temp_data.ts2 {
                            info!(
                                "  TS2: {} °C",
                                ts2_val as f32 / 100.0
                            );
                        }
                        if let Some(ts3_val) = temp_data.ts3 {
                            info!(
                                "  TS3: {} °C",
                                ts3_val as f32 / 100.0
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to convert temperature sensor readings for display: {}",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                error!("Failed to read temperature sensor readings: {:?}", e);
                *temps_ref = None; // Assign None on error via mutable reference
            }
        }

        // Read Current
        let current_ref = &mut current;
        match bq.read_current().await {
            Ok(c) => {
                let current_ma = bq.convert_raw_cc_to_current_ma(c.raw_cc, sense_resistor_uohms);
                info!(
                    "Raw CC: {}, Current: {} mA",
                    c.raw_cc,
                    current_ma // Assuming this is already in mA (e.g., i32)
                );
                *current_ref = Some(current_ma); // Assign to the outer variable via mutable reference
            }
            Err(e) => {
                error!("Failed to read current: {:?}", e);
                *current_ref = None; // Assign None on error via mutable reference
            }
        }

        // Read System Status
        let system_status_ref = &mut system_status;
        match bq.read_status().await {
            Ok(status) => {
                info!("System Status:");
                info!(
                    "  CC Ready: {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY)
                );
                // info!("  Overtemperature: {}", status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP)); // Removed OVR_TEMP check
                info!(
                    "  Undervoltage (UV): {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::UV)
                );
                info!(
                    "  Overvoltage (OV): {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::OV)
                );
                info!(
                    "  Short Circuit Discharge (SCD): {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::SCD)
                );
                info!(
                    "  Overcurrent Discharge (OCD): {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::OCD)
                );
                info!(
                    "  Device X-Ready: {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY)
                );
                info!(
                    "  Override Alert: {}",
                    status
                        .0
                        .contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT)
                );
                *system_status_ref = Some(status); // Assign to the outer variable via mutable reference

                // Clear status flags after reading
                // Only clear flags that are set
                let flags_to_clear = (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY) as u8 * bq769x0_async_rs::registers::SysStatFlags::CC_READY.bits())
                    // | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP) as u8 * bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP.bits()) // Removed OVR_TEMP check
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY) as u8 * bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY.bits())
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT) as u8 * bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT.bits())
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::UV) as u8 * bq769x0_async_rs::registers::SysStatFlags::UV.bits())
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OV) as u8 * bq769x0_async_rs::registers::SysStatFlags::OV.bits())
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::SCD) as u8 * bq769x0_async_rs::registers::SysStatFlags::SCD.bits())
                    | (status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OCD) as u8 * bq769x0_async_rs::registers::SysStatFlags::OCD.bits());

                if flags_to_clear != 0 {
                    if let Err(e) = bq.clear_status_flags(flags_to_clear).await {
                        error!("Failed to clear status flags: {:?}", e);
                    } else {
                        info!("Cleared status flags: {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(e) => {
                error!("Failed to read system status: {:?}", e);
                *system_status_ref = None; // Assign None on error via mutable reference
            }
        }

        // Read SYS_CTRL2 for MOS status
        let mos_status_ref = &mut mos_status;
        match bq.read_register(Register::SysCtrl2).await {
            Ok(sys_ctrl2_byte) => {
                let mos = bq769x0_async_rs::data_types::MosStatus::new(sys_ctrl2_byte);
                info!("MOS Status:");
                info!(
                    "  Charge ON: {}",
                    mos.0
                        .contains(bq769x0_async_rs::registers::SysCtrl2Flags::CHG_ON)
                );
                info!(
                    "  Discharge ON: {}",
                    mos.0
                        .contains(bq769x0_async_rs::registers::SysCtrl2Flags::DSG_ON)
                );
                *mos_status_ref = Some(mos); // Assign to the outer variable via mutable reference
            }
            Err(e) => {
                error!("Failed to read SYS_CTRL2 for MOS status: {:?}", e);
                *mos_status_ref = None; // Assign None on error via mutable reference
            }
        }

        // 发布 BQ76920 告警信息
        if let Ok(status) = bq.read_status().await {
            let alerts = crate::data_types::Bq76920Alerts {
                system_status: status,
            };
            bq76920_alerts_publisher.publish_immediate(alerts);
        }

        info!("----------------------------");

        // Construct and publish BQ76920 measurements
        let bq76920_measurements = crate::data_types::Bq76920Measurements {
            core_measurements: bq769x0_async_rs::data_types::Bq76920Measurements {
                cell_voltages: voltages
                    .unwrap_or_else(bq769x0_async_rs::data_types::CellVoltages::new),
                temperatures: temps.unwrap_or_else(
                    bq769x0_async_rs::data_types::TemperatureSensorReadings::new,
                ),
                current: current.unwrap_or_else(|| 0i32), // Default to 0 mA
                system_status: system_status
                    .unwrap_or_else(|| bq769x0_async_rs::data_types::SystemStatus::new(0)),
                mos_status: mos_status
                    .unwrap_or_else(|| bq769x0_async_rs::data_types::MosStatus::new(0)), // Use default if read failed
            },
        };

        // Publish BQ76920 measurements
        bq76920_measurements_publisher.publish_immediate(bq76920_measurements); // Added publishing

        // Note: AllMeasurements is no longer constructed and published in this task.
        // measurements_publisher.publish_immediate(all_measurements); // Removed AllMeasurements publishing

        // Wait for 1 second
        Timer::after(Duration::from_secs(1)).await; // Adjust delay as needed
    }
}
