#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    i2c::{self, I2c, Config as I2cConfig},
    peripherals::I2C1,
    time::Hertz,
};
use embassy_time::{Timer, Duration};
use smart_battery_driver::{SmartBattery, Enabled};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C1 => i2c::EventInterruptHandler<I2C1>, i2c::ErrorInterruptHandler<I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("stm32g0 demo: boot");
    let p = embassy_stm32::init(Default::default());

    // Configure I2C1 PB6/PB7 with internal pull-ups
    // Pull-ups are required unless you have external resistors (~4.7k) on the bus.
    let mut i2c_cfg = I2cConfig::default();
    i2c_cfg.scl_pullup = true;
    i2c_cfg.sda_pullup = true;
    // Select bus speed: 100 kHz by default; enable feature "i2c-400k" for 400 kHz
    #[cfg(feature = "i2c-400k")]
    let bus_hz: u32 = 400_000;
    #[cfg(not(feature = "i2c-400k"))]
    let bus_hz: u32 = 100_000;
    let i2c = I2c::new(
        p.I2C1,
        p.PB6,
        p.PB7,
        Irqs,
        p.DMA1_CH1,
        p.DMA1_CH2,
        Hertz(bus_hz),
        i2c_cfg,
    );

    let mut bat: SmartBattery<_, Enabled> = SmartBattery::new(i2c);

    match bat.ping().await {
        Ok(true) => info!("Smart battery signature OK ('SB')"),
        Ok(false) => warn!("Unexpected signature"),
        Err(_e) => warn!("I2C error on ping"),
    }

    loop {
        let v = bat.read_vbat_mv().await;
        let i = bat.read_ibat_ma().await;
        let t = bat.read_tpack_cc().await;
        match (v, i, t) {
            (Ok(v), Ok(i), Ok(t)) => info!("VBAT={} mV IBAT={} mA Tpack={} cC", v, i, t),
            _ => warn!("read telemetry failed"),
        }

        let _ = bat.set_charging_enable(true).await;

        Timer::after(Duration::from_secs(2)).await;
    }
}
