#![no_std]
#![no_main]

mod bq76920_task;
mod data_types;
mod sc8815_task;
mod shared;

use bq769x0_async_rs::{BatteryConfig, Bq769x0, Enabled as BqCrcEnabled, ProtectionConfig};
use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Input, Level, Output, Pull, Speed},
    i2c::{self, Config as I2cConfig, I2c},
    peripherals::I2C2,
    time::Hertz,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

// Fixed I2C address for BQ7692003PWR (7‑bit).
// Per TI device comparison table: the "03" variant uses CRC and a fixed
// I2C address 0x08. Other orderable numbers (e.g. BQ7692006) use 0x18.
const BQ76920_I2C_ADDR: u8 = 0x08;
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
    let ce = Output::new(p.PA10, Level::High, Speed::Low);
    let pstop = Output::new(p.PA9, Level::High, Speed::Low);
    info!("Startup: CE=HIGH (disabled), PSTOP=HIGH (power stage gated)");

    // Prepare INNER I2C bus (I2C2 on PB10/PB11) with 100 kHz clock and wrap as shared bus.
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
    type I2c2Bus = Mutex<CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>;
    static I2C2_BUS: StaticCell<I2c2Bus> = StaticCell::new();
    let i2c_bus: &'static I2c2Bus = I2C2_BUS.init(Mutex::new(i2c));

    // Create control inputs for BQ76920 task (active-low enables). Use pull-ups so default is disabled.
    let pb9_discharge_control = Input::new(p.PB9, Pull::Up);
    let pa1_charge_control = Input::new(p.PA1, Pull::Up);

    // Initialize pubsub channels and get publishers for BQ76920 task
    let (
        _measurements_pub,
        _measurements_chan,
        sc8815_alerts_pub,
        _sc8815_alerts_chan,
        bq76920_alerts_pub,
        _bq76920_alerts_chan,
        sc8815_meas_pub,
        _sc8815_meas_chan,
        bq76920_meas_pub,
        _bq76920_meas_chan,
    ) = shared::init_pubsubs();

    // Gate other tasks on successful BQ76920 initialization using fixed I2C address.
    let _selected_bq_addr = loop {
        info!(
            "Attempting BQ76920 init at fixed I2C addr=0x{:02x}",
            BQ76920_I2C_ADDR
        );
        let i2c_dev_for_bq = I2cDevice::new(i2c_bus);
        let mut bq: Bq769x0<_, BqCrcEnabled, 5> =
            Bq769x0::new(i2c_dev_for_bq, BQ76920_I2C_ADDR, 3, None);
        let cfg = BatteryConfig {
            overvoltage_trip: 3650,  // 3.65V per cell
            undervoltage_trip: 2800, // 2.80V per cell
            protection_config: ProtectionConfig {
                scd_limit: 15_000, // 15A short-circuit
                ocd_limit: 10_000, // 10A overcurrent
                ..BatteryConfig::default().protection_config
            },
            rsense: 3, // 3 mΩ sense resistor
            ..Default::default()
        };

        match bq.try_apply_config(&cfg).await {
            Ok(_) => {
                info!(
                    "BQ76920 initialized and verified at 0x{:02x}",
                    BQ76920_I2C_ADDR
                );
                // Spawn the continuous BQ76920 task now that init succeeded.
                let i2c_dev_runtime = I2cDevice::new(i2c_bus);
                _spawner
                    .spawn(bq76920_task::bq76920_task(
                        i2c_dev_runtime,
                        BQ76920_I2C_ADDR,
                        3,    // sense resistor mΩ
                        None, // no NTC parameters provided
                        pb9_discharge_control,
                        pa1_charge_control,
                        bq76920_alerts_pub,
                        bq76920_meas_pub,
                    ))
                    .ok();
                break BQ76920_I2C_ADDR;
            }
            Err(e) => {
                warn!(
                    "BQ76920 init failed at fixed addr 0x{:02x}: {:?}",
                    BQ76920_I2C_ADDR, e
                );
            }
        }
        warn!("BQ76920 not responding. Retrying in 5s...");
        Timer::after(Duration::from_secs(5)).await;
    };

    // Spawn SC8815 charger management task now that the protection IC is live.
    let i2c_dev_for_sc = I2cDevice::new(i2c_bus);
    _spawner
        .spawn(sc8815_task::sc8815_task(
            ce,
            pstop,
            i2c_dev_for_sc,
            sc8815_task::SC8815_DEFAULT_ADDRESS,
            sc8815_alerts_pub,
            sc8815_meas_pub,
        ))
        .ok();

    // Keep the main task alive while worker tasks run.
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
