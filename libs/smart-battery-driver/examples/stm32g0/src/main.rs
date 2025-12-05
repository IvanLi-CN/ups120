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
use smart_battery_driver::{SmartBattery, Enabled, RegisterAccess};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C1 => i2c::EventInterruptHandler<I2C1>, i2c::ErrorInterruptHandler<I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("stm32g0 demo: boot");
    info!("demo build-ts {}", env!("SB_DEMO_BUILD_TS"));
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
    i2c_cfg.frequency = Hertz(bus_hz);
    let i2c = I2c::new(
        p.I2C1,
        p.PB6,
        p.PB7,
        Irqs,
        p.DMA1_CH1,
        p.DMA1_CH2,
        i2c_cfg,
    );

    let mut bat: SmartBattery<_, Enabled> = SmartBattery::new(i2c);

    // Quick bus scan to verify ACK on 0x35 before higher-level ops
    {
        let mut found: heapless::Vec<u8, 16> = heapless::Vec::new();
        // Use a temporary handle for scanning so we don't move the main `bat`
        let mut scan = SmartBattery::new(bat.release());
        for addr in 0x30u8..=0x3Au8 { // small window around 0x35
            let mut probe = scan.with_addr(addr);
            if RegisterAccess::read_registers(&mut probe, 0, 1).await.is_ok() {
                let _ = found.push(addr);
            }
            // rebuild `scan` from the inner I2C after using the probed instance
            let i2c_back = probe.release();
            scan = SmartBattery::new(i2c_back);
        }
        if found.is_empty() { warn!("I2C scan: no ACK in 0x30..0x3A"); }
        else { info!("I2C scan ACK at addrs: {:?}", found.as_slice()); }
        // rebuild main handle at default address
        let i2c_back = scan.with_addr(0x35).release();
        bat = SmartBattery::new(i2c_back);
    }

    match bat.ping().await {
        Ok(true) => info!("Smart battery signature OK ('SB')"),
        Ok(false) => warn!("Unexpected signature"),
        Err(_e) => warn!("I2C error on ping"),
    }

    loop {
        let v = bat.read_vbat_mv().await;
        let i = bat.read_ibat_ma().await;
        let temps = bat.read_temp_window_i8().await;
        match (v, i, temps) {
            (Ok(v), Ok(i), Ok(ts)) => {
                let tpack = ts[0];
                if tpack == i8::MIN {
                    info!("VBAT={} mV IBAT={} mA Tpack=NA raw_window={:?}", v, i, ts);
                } else {
                    info!(
                        "VBAT={} mV IBAT={} mA Tpack={} C raw_window={:?}",
                        v, i, tpack, ts
                    );
                }
            }
            _ => warn!("read telemetry failed"),
        }

        let _ = bat.set_charge_control(true, true, 1).await;

        Timer::after(Duration::from_secs(2)).await;
    }
}
