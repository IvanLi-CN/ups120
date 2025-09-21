#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    i2c::{self, I2c, Config as I2cConfig},
    peripherals::I2C1,
    time::Hertz,
};
use embassy_time::{Timer, Duration};
use smart_battery_driver::{SmartBattery, Enabled};
use rtt_target::{rtt_init_print, rprintln};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    I2C1 => i2c::EventInterruptHandler<I2C1>, i2c::ErrorInterruptHandler<I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Initialize RTT printing before any logs.
    rtt_init_print!();
    rprintln!("stm32g0 demo: boot");
    let p = embassy_stm32::init(Default::default());

    // Configure I2C1 PB6/PB7 with internal pull-ups similar to sc8815 example
    let i2c_cfg = I2cConfig::default();
    let i2c = I2c::new(
        p.I2C1,
        p.PB6,
        p.PB7,
        Irqs,
        p.DMA1_CH1,
        p.DMA1_CH2,
        Hertz(100_000),
        i2c_cfg,
    );

    let mut bat: SmartBattery<_, Enabled> = SmartBattery::new(i2c);

    match bat.ping().await {
        Ok(true) => rprintln!("Smart battery signature OK ('SB')"),
        Ok(false) => rprintln!("Unexpected signature"),
        Err(_e) => rprintln!("I2C error on ping"),
    }

    loop {
        let v = bat.read_vbat_mv().await;
        let i = bat.read_ibat_ma().await;
        let t = bat.read_tpack_cc().await;
        match (v, i, t) {
            (Ok(v), Ok(i), Ok(t)) => rprintln!("VBAT={} mV IBAT={} mA Tpack={} cC", v, i, t),
            _ => rprintln!("read telemetry failed"),
        }

        let _ = bat.set_charging_enable(true).await;

        Timer::after(Duration::from_secs(2)).await;
    }
}

