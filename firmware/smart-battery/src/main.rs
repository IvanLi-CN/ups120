#![no_std]
#![no_main]

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Level, Output, Speed},
    i2c::{self, Config as I2cConfig, I2c},
    peripherals::I2C2,
    time::Hertz,
};
use embassy_time::{Duration, Timer};
use sc8815::{
    DeadTime, DeviceConfiguration, OperatingMode, SC8815, SwitchingFrequency,
    registers::constants::DEFAULT_ADDRESS,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct I2c2Irqs {
    I2C2 => i2c::EventInterruptHandler<I2C2>, i2c::ErrorInterruptHandler<I2C2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    let p = embassy_stm32::init(config);

    // Keep SC8815 power stage disabled during configuration.
    let mut ce = Output::new(p.PA10, Level::High, Speed::Low);
    let mut pstop = Output::new(p.PA9, Level::High, Speed::Low);
    info!("SC8815 bring-up start: CE=HIGH (disabled), PSTOP=HIGH (power stage gated)");

    // Prepare INNER I2C bus (I2C2 on PB10/PB11) with 100 kHz clock.
    let i2c_config = I2cConfig::default();
    let i2c = I2c::new(
        p.I2C2,
        p.PB10,
        p.PB11,
        I2c2Irqs,
        p.DMA1_CH4,
        p.DMA1_CH5,
        Hertz(100_000),
        i2c_config,
    );

    // Pull CE low to enable the charger before any I2C transaction.
    Timer::after(Duration::from_millis(10)).await;
    ce.set_low();
    info!("CE pulled LOW, waiting 100ms before communicating with SC8815");
    Timer::after(Duration::from_millis(100)).await;

    let mut sc8815 = SC8815::new(i2c, DEFAULT_ADDRESS);

    info!("Initializing SC8815 while PSTOP remains HIGH");
    if let Err(e) = sc8815.init().await {
        error!("Failed to initialize SC8815: {:?}", e);
        ce.set_high();
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
        ce.set_high();
        warn!("Charger disabled due to configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_vbat_monitor_ratio(0).await {
        error!("Failed to set VBAT monitor ratio: {:?}", e);
        ce.set_high();
        warn!("Charger disabled due to VBAT monitor configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_otg_mode(false).await {
        error!("Failed to force charging mode: {:?}", e);
        ce.set_high();
        warn!("Charger disabled after OTG configuration failure");
        return;
    }

    if let Err(e) = sc8815.set_adc_conversion(true).await {
        error!("Failed to start SC8815 ADC conversions: {:?}", e);
        ce.set_high();
        warn!("Charger disabled after ADC configuration failure");
        return;
    }

    info!("Configuration done, keeping PSTOP HIGH for 100ms before enabling power stage");
    Timer::after(Duration::from_millis(100)).await;
    pstop.set_low();
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
                    pstop.set_high();
                    ce.set_high();
                }
            }
            Err(e) => {
                error!("Failed to read SC8815 status: {:?}", e);
                pstop.set_high();
                ce.set_high();
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
            }
            Err(e) => error!("Failed to read SC8815 ADC measurements: {:?}", e),
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
