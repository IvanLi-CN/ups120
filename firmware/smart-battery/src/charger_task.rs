use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use sc8815::{DeadTime, DeviceConfiguration, OperatingMode, SC8815, SwitchingFrequency};

use crate::shared::{
    Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher, Sc8815MeasurementsPublisher,
};

/// Embassy task for managing the SC8815 charger IC.
#[embassy_executor::task]
pub async fn charger_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>>,
    address: u8,
    mut pstop_pin: Output<'static>,
    sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    // Create SC8815 driver instance
    let mut sc8815 = SC8815::new(i2c_bus, address);

    // Initialize the SC8815
    if let Err(_e) = sc8815.init().await {
        error!("sc:init");
        return;
    }

    // Configure SC8815 for charging mode - using external resistor configuration
    let mut config = DeviceConfiguration::default();

    config.battery.use_internal_setting = false;

    // Configure current limits with 10mΩ sense resistors
    config.current_limits.rs1_mohm = 10;
    config.current_limits.rs2_mohm = 10;
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
    if let Err(_e) = sc8815.configure_device(&config).await {
        error!("sc:cfg");
        return;
    }

    // In external mode, manually set VBAT monitor ratio for typical battery voltages
    // Use 12.5x ratio for batteries >10.24V (most Li-ion applications)
    if let Err(_e) = sc8815.set_vbat_monitor_ratio(0).await {
        error!("sc:vbat_ratio");
        return;
    }

    defmt::debug!("SC8815 OK");

    // Enable charging mode (disable OTG mode)
    if let Err(_e) = sc8815.set_otg_mode(false).await {
        error!("sc:chg_mode");
        return;
    }
    defmt::debug!("Charging mode enabled successfully");

    // Enable ADC conversion
    if let Err(_e) = sc8815.set_adc_conversion(true).await {
        error!("sc:adc");
    }

    // SC8815 state tracking
    let sc8815_initialized = true; // Set to true after successful initialization
    let mut sc8815_comm_failed = false; // Track if communication has ever failed

    loop {
        // Get BQ76920 measurements for safety checks
        let _bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;

        // Read SC8815 ADC measurements
        let sc8815_adc_measurements_option = match sc8815.get_adc_measurements().await {
            Ok(measurements) => {
                // Print SC8815 voltage and current information
                debug!("sc vbus={} vbat={} ibus={} ibat={}",
                    measurements.vbus_mv,
                    measurements.vbat_mv,
                    measurements.ibus_ma,
                    measurements.ibat_ma);
                Some(measurements)
            }
            Err(_e) => {
                error!("sc:adc");
                sc8815_comm_failed = true; // Mark communication failure
                None
            }
        };

        // Read SC8815 device status
        let sc8815_status_option = match sc8815.get_device_status().await {
            Ok(status) => {
                // Check for critical faults
                if status.otp_fault {
                    defmt::debug!("sc:otp");
                }
                if status.vbus_short_fault {
                    defmt::debug!("sc:vbus_short");
                }
                Some(status)
            }
            Err(_e) => {
                error!("sc:status");
                sc8815_comm_failed = true; // Mark communication failure
                None
            }
        };

        // Charging control with safety checks
        let _bq76920_measurements = _bq76920_measurements;

        let can_charge = sc8815_initialized && !sc8815_comm_failed;

        if can_charge {
            if let Some(measurements) = sc8815_adc_measurements_option.as_ref() {
                if measurements.vbat_mv < 18000 {
                    pstop_pin.set_low();
                    debug!("sc:pstop L");
                    if let Err(_) = sc8815.set_ibat_limit(500, 0, 10).await {
                        sc8815_comm_failed = true;
                    }
                    if let Err(_) = sc8815.set_otg_mode(false).await {
                        sc8815_comm_failed = true;
                    }
                    // Log charging status every 10 seconds
                    static mut LAST_LOG_TIME: u32 = 0;
                    let current_time = embassy_time::Instant::now().as_millis() as u32;
                    unsafe {
                        if current_time - LAST_LOG_TIME > 10000 {
                            debug!("chg v={} i={}", measurements.vbat_mv, measurements.ibat_ma);
                            LAST_LOG_TIME = current_time;
                        }
                    }
                } else {
                    pstop_pin.set_high();
                    defmt::debug!("chg:ov {}>=18000", measurements.vbat_mv);
                }
            } else {
                pstop_pin.set_high();
                defmt::debug!("chg:no-meas");
            }
        } else {
            pstop_pin.set_high();
            defmt::debug!("chg:no init={} comm_ok={}", sc8815_initialized, !sc8815_comm_failed);
        }

        // Publish measurements and alerts
        if let Some(adc_measurements) = sc8815_adc_measurements_option {
            let sc8815_measurements_payload =
                crate::data_types::Sc8815Measurements { adc_measurements };
            sc8815_measurements_publisher.publish_immediate(sc8815_measurements_payload);
        }

        if let Some(status) = sc8815_status_option {
            let alerts = crate::data_types::Sc8815Alerts {
                device_status: status,
                expected_charging: false,
                charging_confirmed: false,
                ov_pause_active: false,
                imbalance_pause_active: false,
            };
            sc8815_alerts_publisher.publish_immediate(alerts);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
