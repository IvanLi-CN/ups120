use defmt::{error, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use sc8815::{DeadTime, DeviceConfiguration, OperatingMode, SC8815, SwitchingFrequency};

use crate::data_types::{Sc8815Alerts, Sc8815Measurements};
use crate::shared::{Sc8815AlertsPublisher, Sc8815MeasurementsPublisher};

pub const SC8815_DEFAULT_ADDRESS: u8 = sc8815::registers::constants::DEFAULT_ADDRESS;

/// Embassy task managing the SC8815 charger with safety gating.
#[embassy_executor::task]
pub async fn sc8815_task(
    mut ce_pin: Output<'static>,
    mut pstop_pin: Output<'static>,
    i2c_device: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_stm32::mode::Async>,
    >,
    address: u8,
    sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
) {
    // Ensure charger is disabled until configuration completes.
    ce_pin.set_high();
    pstop_pin.set_high();
    info!("SC8815 task starting: CE=HIGH (disabled), PSTOP=HIGH (power stage gated)");

    Timer::after(Duration::from_millis(10)).await;
    ce_pin.set_low();
    info!("CE pulled LOW, waiting 100ms before communicating with SC8815");
    Timer::after(Duration::from_millis(100)).await;

    let mut sc8815 = SC8815::new(i2c_device, address);

    info!("Initializing SC8815 while PSTOP remains HIGH");
    if let Err(e) = sc8815.init().await {
        error!("Failed to initialize SC8815: {:?}", e);
        ce_pin.set_high();
        warn!("Charger disabled due to initialization failure");
        return;
    }

    let mut device_config = DeviceConfiguration::default();
    device_config.battery.use_internal_setting = false; // External divider: Ru=140kΩ, Rd=10kΩ → ~18V target
    device_config.current_limits.rs1_mohm = 5;
    device_config.current_limits.rs2_mohm = 5;
    device_config.current_limits.ibus_limit_ma = 800;
    device_config.current_limits.ibat_limit_ma = 800;
    device_config.power.operating_mode = OperatingMode::Charging;
    device_config.power.switching_frequency = SwitchingFrequency::Freq450kHz;
    device_config.power.dead_time = DeadTime::Ns60;
    device_config.power.vinreg_voltage_mv = 11500;
    device_config.trickle_charging = true;
    device_config.charging_termination = true;
    device_config.use_ibus_for_charging = false;

    info!("Applying SC8815 charger configuration (5mΩ sensors, 800mA limits)");
    if let Err(e) = sc8815.configure_device(&device_config).await {
        error!("Failed to configure SC8815: {:?}", e);
        ce_pin.set_high();
        warn!("Charger disabled due to configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_vbat_monitor_ratio(0).await {
        error!("Failed to set VBAT monitor ratio: {:?}", e);
        ce_pin.set_high();
        warn!("Charger disabled due to VBAT monitor configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_otg_mode(false).await {
        error!("Failed to force charging mode: {:?}", e);
        ce_pin.set_high();
        warn!("Charger disabled after OTG configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_adc_conversion(true).await {
        error!("Failed to start SC8815 ADC conversions: {:?}", e);
        ce_pin.set_high();
        warn!("Charger disabled after ADC configuration failure");
        return;
    }

    info!("Configuration done, keeping PSTOP HIGH for 100ms before enabling power stage");
    Timer::after(Duration::from_millis(100)).await;
    pstop_pin.set_low();
    info!("PSTOP pulled LOW, SC8815 power stage enabled");

    loop {
        match sc8815.get_device_status().await {
            Ok(status) => {
                info!(
                    "SC8815 status -> AC:{} USB_LOAD:{} Faults: OTP={} VBUS_SHORT={}",
                    status.ac_adapter_connected,
                    status.usb_load_detected,
                    status.otp_fault,
                    status.vbus_short_fault
                );

                if status.otp_fault || status.vbus_short_fault {
                    warn!("SC8815 reported fault, keeping PSTOP HIGH for safety");
                    pstop_pin.set_high();
                    ce_pin.set_high();
                }

                let alerts_payload = Sc8815Alerts {
                    device_status: status,
                };
                sc8815_alerts_publisher.publish_immediate(alerts_payload);
            }
            Err(e) => {
                error!("Failed to read SC8815 status: {:?}", e);
                pstop_pin.set_high();
                ce_pin.set_high();
            }
        }

        match sc8815.get_adc_measurements().await {
            Ok(measurements) => {
                info!(
                    "VBUS={}mV VBAT={}mV IBUS={}mA IBAT={}mA",
                    measurements.vbus_mv,
                    measurements.vbat_mv,
                    measurements.ibus_ma,
                    measurements.ibat_ma
                );
                let meas_payload = Sc8815Measurements {
                    adc_measurements: measurements,
                };
                sc8815_measurements_publisher.publish_immediate(meas_payload);
            }
            Err(e) => error!("Failed to read SC8815 ADC measurements: {:?}", e),
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
