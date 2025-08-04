//! SC8815 充电器任务模块
//!
//! 负责管理SC8815充电器IC的配置、监控和控制功能

use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use sc8815::{DeadTime, DeviceConfiguration, OperatingMode, SC8815, SwitchingFrequency};

use crate::shared::{
    Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher, Sc8815MeasurementsPublisher,
};

/// Embassy task for managing the SC8815 charger IC.
#[embassy_executor::task]
pub async fn charger_task(
    i2c_bus: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_rp::peripherals::I2C0, embassy_rp::i2c::Async>,
    >,
    address: u8,
    mut pstop_pin: Output<'static>,
    sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    info!("Charger task started");

    // Create charger driver instance
    let mut charger = SC8815::new(i2c_bus, address);

    // Initialize the charger
    if let Err(_e) = charger.init().await {
        error!("Failed to initialize charger");
        return;
    }

    // Configure SC8815 for charging mode - using external resistor configuration
    let mut config = DeviceConfiguration::default();

    config.battery.use_internal_setting = false;

    // Configure current limits with 5mΩ sense resistors (as per reference)
    config.current_limits.rs1_mohm = 5;
    config.current_limits.rs2_mohm = 5;
    config.current_limits.ibus_limit_ma = 500; // 500mA input current
    config.current_limits.ibat_limit_ma = 500; // 500mA charging current

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
    if let Err(_e) = charger.configure_device(&config).await {
        error!("Failed to configure charger");
        return;
    }

    // In external mode, manually set VBAT monitor ratio for typical battery voltages
    // Use 12.5x ratio for batteries >10.24V (most Li-ion applications)
    if let Err(_e) = charger.set_vbat_monitor_ratio(0).await {
        error!("Failed to set VBAT monitor ratio");
        return;
    }

    info!("Charger configuration applied successfully");

    // Enable charging mode (disable OTG mode)
    if let Err(_e) = charger.set_otg_mode(false).await {
        error!("Failed to enable charging mode");
        return;
    }
    info!("Charging mode enabled successfully");

    // Enable ADC conversion
    if let Err(_e) = charger.set_adc_conversion(true).await {
        error!("Failed to start charger ADC conversion");
    }

    info!("Charger initialization complete");

    // Charger state tracking
    let charger_initialized = true; // Set to true after successful initialization
    let mut charger_comm_failed = false; // Track if communication has ever failed

    loop {
        // Get BQ76920 measurements for safety checks
        let _bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;

        // Read charger ADC measurements
        let charger_adc_measurements_option = match charger.get_adc_measurements().await {
            Ok(measurements) => {
                // Print charger voltage and current information
                info!(
                    "[CHARGE] VBUS:{}mV, VBAT:{}mV, IBUS:{}mA, IBAT:{}mA",
                    measurements.vbus_mv,
                    measurements.vbat_mv,
                    measurements.ibus_ma,
                    measurements.ibat_ma
                );
                Some(measurements)
            }
            Err(_e) => {
                error!("[CHARGER] Failed to read ADC measurements");
                charger_comm_failed = true; // Mark communication failure
                None
            }
        };

        // Read charger device status
        let charger_status_option = match charger.get_device_status().await {
            Ok(status) => {
                // Check for critical faults
                if status.otp_fault {
                    warn!("[CHARGER] Over-temperature protection fault detected!");
                }
                if status.vbus_short_fault {
                    warn!("[CHARGE] VBUS short circuit fault detected!");
                }
                Some(status)
            }
            Err(_e) => {
                error!("Failed to read charger device status");
                charger_comm_failed = true; // Mark communication failure
                None
            }
        };

        // Charging control with safety checks
        // BQ76920 measurements are available for safety checks if needed

        let can_charge = charger_initialized && !charger_comm_failed;

        if can_charge {
            if let Some(measurements) = charger_adc_measurements_option.as_ref() {
                if measurements.vbat_mv < 18000 {
                    pstop_pin.set_low();
                    info!("[DEBUG] PSTOP set to LOW - charging should be enabled");
                    if (charger.set_ibat_limit(500, 0, 5).await).is_err() {
                        charger_comm_failed = true;
                    }
                    if (charger.set_otg_mode(false).await).is_err() {
                        charger_comm_failed = true;
                    }
                    // Log charging status every 10 seconds
                    static mut LAST_LOG_TIME: u32 = 0;
                    let current_time = embassy_time::Instant::now().as_millis() as u32;
                    unsafe {
                        if current_time - LAST_LOG_TIME > 10000 {
                            info!(
                                "[CHARGING] VBAT:{}mV, IBAT:{}mA, Status: Active",
                                measurements.vbat_mv, measurements.ibat_ma
                            );
                            LAST_LOG_TIME = current_time;
                        }
                    }
                } else {
                    pstop_pin.set_high();
                    warn!(
                        "[CHARGING] Voltage too high: {}mV >= 18000mV",
                        measurements.vbat_mv
                    );
                }
            } else {
                pstop_pin.set_high();
                warn!("[CHARGING] No measurements available");
            }
        } else {
            pstop_pin.set_high();
            warn!(
                "[CHARGING] Cannot charge - init:{} comm_ok:{}",
                charger_initialized, !charger_comm_failed
            );
        }

        // Publish measurements and alerts
        if let Some(adc_measurements) = charger_adc_measurements_option {
            let charger_measurements_payload =
                crate::data_types::Sc8815Measurements { adc_measurements };
            sc8815_measurements_publisher.publish_immediate(charger_measurements_payload);
        }

        if let Some(status) = charger_status_option {
            let alerts = crate::data_types::Sc8815Alerts {
                device_status: status,
            };
            sc8815_alerts_publisher.publish_immediate(alerts);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
