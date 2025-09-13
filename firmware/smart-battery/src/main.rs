#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Enable external low-speed clock (LSE, 32.768 kHz) for accurate timing
    let mut config = embassy_stm32::Config::default();
    config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    let _p = embassy_stm32::init(config);

    loop {
        defmt::info!("Hello, world from smart-battery");
        Timer::after(Duration::from_secs(1)).await;
    }
}
