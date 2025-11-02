#![no_std]
#![no_main]

mod i2c_slave;

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    i2c::{self, Config as I2cConfig, I2c, SlaveAddrConfig},
    interrupt::typelevel::Interrupt,
    peripherals::I2C1,
    time::Hertz,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct I2c1Irqs {
    I2C1 => i2c::EventInterruptHandler<I2C1>, i2c::ErrorInterruptHandler<I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    config.enable_debug_during_sleep = false;
    let p = embassy_stm32::init(config);

    unsafe {
        embassy_stm32::interrupt::typelevel::I2C1::unpend();
        embassy_stm32::interrupt::typelevel::I2C1::enable();
    }

    let mut i2c_cfg = I2cConfig::default();
    i2c_cfg.frequency = Hertz(100_000);
    let i2c1 = I2c::new_blocking(p.I2C1, p.PB6, p.PB7, i2c_cfg);
    let i2c1_slave = i2c1.into_slave_multimaster(SlaveAddrConfig::basic(i2c_slave::SLAVE_ADDRESS));

    info!(
        "stm32: i2c slave ready (addr=0x{:02x}, build_ts={})",
        i2c_slave::SLAVE_ADDRESS,
        env!("SB_BUILD_TS")
    );
    let token = i2c_slave::task(i2c1_slave).expect("allocate I2C slave task");
    _spawner.spawn(token);

    core::future::pending::<()>().await;
}
