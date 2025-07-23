use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{i2c::I2c, gpio::Output};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use sc8815::{
    CellCount, DeviceConfiguration, OperatingMode,
    SwitchingFrequency, VoltagePerCell, DeadTime,
    SC8815,
};

use bq769x0_async_rs::registers::{
    SysCtrl2Flags as Bq76920SysCtrl2Flags, SysStatFlags as Bq76920SysStatFlags,
};

use crate::shared::{Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher, Sc8815MeasurementsPublisher};

// SC8815 charging configuration following reference example

/// Embassy task for managing the SC8815 charger IC.
#[embassy_executor::task]
pub async fn sc8815_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    mut pstop_pin: Output<'static>,
    sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    info!("SC8815 task started.");

    // Create SC8815 driver instance
    let mut sc8815 = SC8815::new(i2c_bus, address);

    // Initialize the SC8815
    if let Err(e) = sc8815.init().await {
        error!("Failed to initialize SC8815: {:?}", e);
        return;
    }

    // Configure SC8815 for charging mode - using original working configuration
    let mut config = DeviceConfiguration::default();

    // Configure for 4S battery using internal voltage setting
    config.battery.cell_count = CellCount::Cells4S;
    config.battery.voltage_per_cell = VoltagePerCell::Mv4450;
    config.battery.use_internal_setting = true;

    // Configure current limits with 5mΩ sense resistors (as per reference)
    config.current_limits.rs1_mohm = 5;
    config.current_limits.rs2_mohm = 5;
    config.current_limits.ibus_limit_ma = 300;  // Minimum allowed value (300mA)
    config.current_limits.ibat_limit_ma = 300;  // Minimum allowed value (300mA)

    // Configure power settings (as per reference)
    config.power.operating_mode = OperatingMode::Charging;
    config.power.switching_frequency = SwitchingFrequency::Freq450kHz;
    config.power.dead_time = DeadTime::Ns60;
    config.power.vinreg_voltage_mv = 11500; // 11.5V VINREG voltage

    // Charger mode settings (as per reference)
    config.trickle_charging = true;
    config.charging_termination = true;
    config.use_ibus_for_charging = false; // Use IBAT (battery side) for charging current

    // Apply configuration
    if let Err(e) = sc8815.configure_device(&config).await {
        error!("Failed to configure SC8815: {:?}", e);
        return;
    }

    info!("SC8815 configured successfully for charging mode");

    // Print configuration details
    info!("[SC8815 Config] Battery: 4S, 4.45V/cell (17.8V total)");
    info!("[SC8815 Config] Current limits: IBUS=300mA, IBAT=300mA (minimum allowed)");
    info!("[SC8815 Config] Sense resistors: RS1=5mΩ, RS2=5mΩ");
    info!("[SC8815 Config] VINREG=11.5V, Switching freq=450kHz");

    // Enable charging mode (disable OTG mode)
    if let Err(e) = sc8815.set_otg_mode(false).await {
        error!("Failed to enable charging mode: {:?}", e);
        return;
    }
    info!("Charging mode enabled successfully");

    // Enable ADC conversion - THIS IS CRITICAL!
    if let Err(e) = sc8815.set_adc_conversion(true).await {
        error!("Failed to start SC8815 ADC conversion: {:?}", e);
    } else {
        info!("[SC8815] ADC conversion enabled successfully");
    }

    loop {
        // Get BQ76920 measurements for safety checks
        let bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;

        // Read SC8815 ADC measurements
        let sc8815_adc_measurements_option = match sc8815.get_adc_measurements().await {
            Ok(measurements) => {
                // Print SC8815 voltage and current information
                info!("[SC8815] VBUS:{}mV, VBAT:{}mV, IBUS:{}mA, IBAT:{}mA",
                      measurements.vbus_mv, measurements.vbat_mv,
                      measurements.ibus_ma, measurements.ibat_ma);
                Some(measurements)
            },
            Err(e) => {
                error!("[SC8815] Failed to read ADC measurements: {:?}", e);
                None
            }
        };

        // Read SC8815 device status
        let sc8815_status_option = match sc8815.get_device_status().await {
            Ok(status) => {
                // Check for critical faults
                if status.otp_fault {
                    warn!("[SC8815] Over-temperature protection fault detected!");
                }
                if status.vbus_short_fault {
                    warn!("[SC8815] VBUS short circuit fault detected!");
                }
                Some(status)
            }
            Err(e) => {
                error!("Failed to read SC8815 device status: {:?}", e);
                None
            }
        };

        // Safety checks based on BQ76920 status
        let bq76920_mos_status = bq76920_measurements.core_measurements.mos_status;
        let bq76920_sys_status = bq76920_measurements.core_measurements.system_status;

        let bq76920_charge_fet_enabled =
            bq76920_mos_status.0.contains(Bq76920SysCtrl2Flags::CHG_ON);
        let bq76920_safe_to_charge = !bq76920_sys_status.0.intersects(Bq76920SysStatFlags::OV);

        let final_charge_permission = bq76920_charge_fet_enabled && bq76920_safe_to_charge;

        // Control charging based on safety conditions using PSTOP pin
        if final_charge_permission {
            // Enable charging by pulling PSTOP low
            pstop_pin.set_low();

            // Set charging current using correct sense resistor value (5mΩ) - 300mA (minimum allowed)
            if let Err(e) = sc8815.set_ibat_limit(300, 0, 5).await {
                error!("[SC8815] Failed to set charge current: {:?}", e);
            }

            // Ensure we're in charging mode
            if let Err(e) = sc8815.set_otg_mode(false).await {
                error!("[SC8815] Failed to set charging mode: {:?}", e);
            }
        } else {
            // Disable charging by pulling PSTOP high
            pstop_pin.set_high();
        }

        // Publish measurements and alerts
        if let Some(adc_measurements) = sc8815_adc_measurements_option {
            let sc8815_measurements_payload = crate::data_types::Sc8815Measurements {
                adc_measurements,
            };
            sc8815_measurements_publisher.publish_immediate(sc8815_measurements_payload);
        }

        if let Some(status) = sc8815_status_option {
            let alerts = crate::data_types::Sc8815Alerts {
                device_status: status,
            };
            sc8815_alerts_publisher.publish_immediate(alerts);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
