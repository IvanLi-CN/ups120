#![no_std]
#![no_main]

#[cfg(not(feature = "ship-mode"))]
mod bq76920_task;
#[cfg(not(feature = "ship-mode"))]
mod data_types;
// mod global_state; // removed to save flash; LEDs derive state locally
#[cfg(not(feature = "ship-mode"))]
mod activity;
#[cfg(not(feature = "ship-mode"))]
mod charger_control;
mod failsafe;
#[cfg(not(feature = "ship-mode"))]
mod i2c_slave;
#[cfg(not(feature = "ship-mode"))]
mod irq_mux;
#[cfg(not(feature = "ship-mode"))]
mod leds4_task;
#[cfg(not(feature = "ship-mode"))]
mod ntc_temp;
#[cfg(not(feature = "ship-mode"))]
mod sc8815_task;
#[cfg(not(feature = "ship-mode"))]
mod state_bits;
#[cfg(not(feature = "ship-mode"))]
mod tmp75;
// scheduler removed (合并到 failsafe + SC 路径)
#[cfg(not(feature = "ship-mode"))]
mod shared;
#[cfg(not(feature = "ship-mode"))]
mod sleep_manager;
#[cfg(not(feature = "ship-mode"))]
mod thermal;

use bq769x0_async_rs::{BatteryConfig, Bq769x0, Enabled as BqCrcEnabled};
// no direct info! logs to减小尺寸
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
#[cfg(not(feature = "ship-mode"))]
use embassy_stm32::exti::ExtiInput;
#[cfg(not(feature = "ship-mode"))]
use embassy_stm32::gpio::Pull;
#[cfg(not(feature = "ship-mode"))]
use embassy_stm32::i2c::SlaveAddrConfig;
use embassy_stm32::interrupt::typelevel::Interrupt as _;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Level, Output, Speed},
    i2c::{self, Config as I2cConfig, I2c},
    peripherals::{I2C1, I2C2},
    time::Hertz,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

#[cfg(not(feature = "ship-mode"))]
use shared::{Bq76920MeasurementsSubscriber, Sc8815MeasurementsSubscriber};

type RuntimeI2c = I2c<'static, embassy_stm32::mode::Async, i2c::mode::Master>;
type SharedI2cBus = Mutex<CriticalSectionRawMutex, RuntimeI2c>;

static I2C2_BUS: StaticCell<SharedI2cBus> = StaticCell::new();

#[cfg(feature = "ship-mode")]
const LED_CYCLE_MS: u64 = 300;
#[cfg(feature = "ship-mode")]
const LED_FLASH_MS: u64 = 250;

#[cfg(not(feature = "ship-mode"))]
const SHIP_BUTTON_HOLD_MS: u64 = 5_000;

#[cfg(feature = "ship-mode")]
fn led_on(pin: &mut Output<'static>) {
    pin.set_low();
}

#[cfg(feature = "ship-mode")]
fn led_off(pin: &mut Output<'static>) {
    pin.set_high();
}

async fn enter_ship_mode_now(
    i2c_bus: &'static SharedI2cBus,
    address: u8,
    sense_resistor_m_ohm: u32,
) -> bool {
    let i2c_dev = I2cDevice::new(i2c_bus);
    let mut bq: Bq769x0<I2cDevice<'static, CriticalSectionRawMutex, RuntimeI2c>, BqCrcEnabled, 5> =
        Bq769x0::new(i2c_dev, address, sense_resistor_m_ohm, None);
    defmt::info!("bq:ship enter");
    match bq.enter_ship_mode().await {
        Ok(_) => {
            defmt::info!("bq:ship ok");
            crate::failsafe::set_bq_online(false);
            true
        }
        Err(e) => {
            defmt::warn!("bq:ship err: {:?}", e);
            false
        }
    }
}

#[cfg(not(feature = "ship-mode"))]
#[embassy_executor::task]
async fn ship_button_task(
    mut button: ExtiInput<'static>,
    i2c_bus: &'static SharedI2cBus,
    address: u8,
) {
    loop {
        button.wait_for_rising_edge().await;
        // 简单消抖
        Timer::after(Duration::from_millis(30)).await;
        if !button.is_high() {
            continue;
        }

        let pressed_at = Instant::now();
        let deadline = pressed_at + Duration::from_millis(SHIP_BUTTON_HOLD_MS);
        while button.is_high() && Instant::now() < deadline {
            Timer::after(Duration::from_millis(50)).await;
        }

        if button.is_high() {
            defmt::info!("btn:ship long-press detected");
            crate::failsafe::request_pstop();
            let ship_ok = enter_ship_mode_now(i2c_bus, address, 3).await;
            defmt::info!("btn:ship result={}", ship_ok);
        }

        // 等待松开，避免重复触发
        button.wait_for_falling_edge().await;
    }
}

#[cfg(not(feature = "ship-mode"))]
#[embassy_executor::task]
async fn sc_meas_mirror_task(mut sub: Sc8815MeasurementsSubscriber<'static>) {
    loop {
        let meas = sub.next_message_pure().await;
        i2c_slave::update_sc_measurements(&meas);
    }
}

#[cfg(not(feature = "ship-mode"))]
#[embassy_executor::task]
async fn bq_meas_mirror_task(mut sub: Bq76920MeasurementsSubscriber<'static, 5>) {
    loop {
        let meas = sub.next_message_pure().await;
        i2c_slave::update_bq_measurements(&meas);
    }
}

// Fixed I2C address for BQ7692003PWR (7‑bit).
// Per TI device comparison table: the "03" variant uses CRC and a fixed
// I2C address 0x08. Other orderable numbers (e.g. BQ7692006) use 0x18.
const BQ76920_I2C_ADDR: u8 = 0x08;
use {defmt_rtt as _, panic_probe as _};

// (no build-id marker to save flash)

bind_interrupts!(struct I2c2Irqs {
    I2C2 => i2c::EventInterruptHandler<I2C2>, i2c::ErrorInterruptHandler<I2C2>;
});

// Bind I2C1 interrupts so the slave listen/respond paths can wake properly.
bind_interrupts!(struct I2c1Irqs {
    I2C1 => i2c::EventInterruptHandler<I2C1>, i2c::ErrorInterruptHandler<I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    // Single firmware profile: keep debug disabled during sleep for lowest current.
    config.enable_debug_during_sleep = false;
    let p = embassy_stm32::init(config);

    // 使用默认线程模式执行器：WFE 进入轻度 SLEEP（非 STOP）。
    // 启动日志（必须打印，复用与 sleep_task 相同的格式字符串以节省 FLASH）。
    defmt::debug!("sleep: start (mode=SLEEP)");

    // (Removed: APB1SMENR diagnostics to save flash)

    #[cfg(not(feature = "ship-mode"))]
    {
        // Configure external I2C1 slave (PB6/PB7 @0x35) using embassy API only.
        // Ensure NVIC line is unmasked so I2C interrupts can wake from SLEEP.
        unsafe {
            #[allow(non_snake_case)]
            {
                // Type-level NVIC handle is generated by embassy for this MCU.
                embassy_stm32::interrupt::typelevel::I2C1::unpend();
                embassy_stm32::interrupt::typelevel::I2C1::enable();
            }
        }
    }
    #[cfg(not(feature = "ship-mode"))]
    let i2c1_dev = {
        let mut i2c1_cfg = I2cConfig::default();
        i2c1_cfg.frequency = Hertz(100_000);
        i2c1_cfg.timeout = Duration::from_millis(150);
        let i2c1_blocking = I2c::new_blocking(p.I2C1, p.PB6, p.PB7, i2c1_cfg);
        // (Optional: I2C1.CR1.WUPEN for wake-from-STOP is omitted to save flash; RTC remains primary wake source.)
        i2c1_blocking.into_slave_multimaster(SlaveAddrConfig::basic(i2c_slave::SLAVE_ADDRESS))
    };

    // Keep SC8815 power stage disabled during configuration.
    #[allow(unused_mut)]
    let mut ce = Output::new(p.PA10, Level::High, Speed::Low);
    #[allow(unused_mut)]
    let mut pstop = Output::new(p.PA9, Level::High, Speed::Low);
    // EXIT_SHIPMODE uses PH0, which is wired (via D3 clamp) onto the BQ76920 TS1 pin.
    // On this hardware the BQ cannot be woken from SHIP by I2C traffic alone; we must
    // assert PH0 high long enough for the analog front end to exit ship mode.
    #[allow(unused_mut)]
    let mut exit_ship = Output::new(p.PH0, Level::Low, Speed::Low);
    defmt::debug!("bq:wake");

    // (Removed: raw I2C1 PAC readbacks to save flash)

    // Prepare INNER I2C bus (I2C2 on PB10/PB11) with 100 kHz clock and wrap as shared bus.
    // Ensure I2C2 NVIC is unmasked to avoid async transactions stalling.
    unsafe {
        #[allow(non_snake_case)]
        {
            embassy_stm32::interrupt::typelevel::I2C2::unpend();
            embassy_stm32::interrupt::typelevel::I2C2::enable();
        }
    }
    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = Hertz(100_000);
    let i2c = I2c::new(
        p.I2C2, p.PB10, p.PB11, I2c2Irqs, p.DMA1_CH4, p.DMA1_CH5, i2c_config,
    );
    let i2c_bus: &'static SharedI2cBus = I2C2_BUS.init(Mutex::new(i2c));

    #[cfg(not(feature = "ship-mode"))]
    let (
        _measurements_pub,
        _measurements_chan,
        sc8815_alerts_pub,
        _sc8815_alerts_chan,
        bq76920_alerts_pub,
        _bq76920_alerts_chan,
        sc8815_meas_pub,
        sc8815_meas_chan,
        bq76920_meas_pub,
        bq76920_meas_chan,
        balancing_cv_pub,
        balancing_cv_chan,
    ) = shared::init_pubsubs();

    // Bring up BQ76920 with a two-phase sequence:
    // 1) Try configuration over I2C at the known addresses.
    // 2) If that fails, assert EXIT_SHIPMODE (PH0 → TS1) high for 500 ms and retry once.
    defmt::debug!("fw:boot smart-battery");
    defmt::debug!("bq:init");
    let tried_addresses = [BQ76920_I2C_ADDR, 0x18u8];
    let mut cfg_template = BatteryConfig {
        overvoltage_trip: 3650,
        undervoltage_trip: 2500,
        rsense: 3,
        ..BatteryConfig::default()
    };
    cfg_template.protection_config.scd_limit = 15_000;
    cfg_template.protection_config.ocd_limit = 10_000;

    let mut bq_init_addr: Option<u8> = None;
    let mut did_exit_ship_pulse = false;

    'outer: loop {
        // Phase: pure I2C configuration attempts.
        for addr in tried_addresses.iter().copied() {
            defmt::debug!("bq:try=0x{:02x}", addr);
            let i2c_dev_for_bq = I2cDevice::new(i2c_bus);
            let mut probe: Bq769x0<_, BqCrcEnabled, 5> =
                Bq769x0::new(i2c_dev_for_bq, addr, 3, None);
            if probe.try_apply_config(&cfg_template).await.is_ok() {
                defmt::debug!("bq:cfg ok addr=0x{:02x}", addr);
                crate::failsafe::set_bq_online(true);
                bq_init_addr = Some(addr);
                break 'outer;
            }
        }

        // If we've already pulsed EXIT_SHIPMODE once, give up after the second round of attempts.
        if did_exit_ship_pulse {
            break;
        }

        // First round failed: drive PH0 high (through D3 to TS1) to force the BQ out of SHIP mode.
        defmt::warn!("bq:init failed, pulsing EXIT_SHIPMODE (PH0→TS1)");
        exit_ship.set_high();
        Timer::after(Duration::from_millis(500)).await;
        exit_ship.set_low();
        did_exit_ship_pulse = true;
        // Loop back and retry configuration at both addresses.
    }

    if bq_init_addr.is_none() {
        crate::failsafe::set_bq_online(false);
        defmt::warn!("bq:init failed after EXIT_SHIPMODE pulse");
    }

    let bq_runtime_addr = bq_init_addr.unwrap_or(BQ76920_I2C_ADDR);

    #[cfg(feature = "ship-mode")]
    {
        ce.set_low();
        pstop.set_high();

        let mut led_r = Output::new(p.PA5, Level::High, Speed::Low);
        let mut led_y = Output::new(p.PA6, Level::High, Speed::Low);
        let mut led_g = Output::new(p.PA7, Level::High, Speed::Low);
        let mut led_b = Output::new(p.PB0, Level::High, Speed::Low);

        let ship_mode_ok = enter_ship_mode_now(i2c_bus, bq_runtime_addr, 3).await;

        if ship_mode_ok {
            loop {
                led_on(&mut led_r);
                led_off(&mut led_y);
                led_off(&mut led_g);
                led_off(&mut led_b);
                Timer::after(Duration::from_millis(LED_CYCLE_MS)).await;

                led_off(&mut led_r);
                led_on(&mut led_y);
                led_off(&mut led_g);
                led_off(&mut led_b);
                Timer::after(Duration::from_millis(LED_CYCLE_MS)).await;

                led_off(&mut led_r);
                led_off(&mut led_y);
                led_on(&mut led_g);
                led_off(&mut led_b);
                Timer::after(Duration::from_millis(LED_CYCLE_MS)).await;

                led_off(&mut led_r);
                led_off(&mut led_y);
                led_off(&mut led_g);
                led_on(&mut led_b);
                Timer::after(Duration::from_millis(LED_CYCLE_MS)).await;
            }
        } else {
            loop {
                led_on(&mut led_r);
                led_on(&mut led_y);
                led_on(&mut led_g);
                led_on(&mut led_b);
                Timer::after(Duration::from_millis(LED_FLASH_MS)).await;

                led_off(&mut led_r);
                led_off(&mut led_y);
                led_off(&mut led_g);
                led_off(&mut led_b);
                Timer::after(Duration::from_millis(LED_FLASH_MS)).await;
            }
        }
    }

    #[cfg(not(feature = "ship-mode"))]
    {
        let ce = ce;
        let pstop = pstop;
        // 初始化完成后再启动相关任务（IRQ → LED → SC → BQ main）
        let sc_int = ExtiInput::new(p.PB2, p.EXTI2, Pull::Up);
        let bq_alert = ExtiInput::new(p.PB1, p.EXTI1, Pull::None);
        let ship_button = ExtiInput::new(p.PB8, p.EXTI8, Pull::None);
        _spawner.spawn(irq_mux::irq_mux_task(sc_int, bq_alert).expect("irq-mux token"));

        let led_r = Output::new(p.PA5, Level::High, Speed::Low);
        let led_y = Output::new(p.PA6, Level::High, Speed::Low);
        let led_g = Output::new(p.PA7, Level::High, Speed::Low);
        let led_b = Output::new(p.PB0, Level::High, Speed::Low);
        let led_bq_sub = bq76920_meas_chan
            .subscriber()
            .expect("Allocate BQ76920 measurements subscriber for 4-LED task");
        let led_sc_alerts_sub = _sc8815_alerts_chan
            .subscriber()
            .expect("Allocate SC8815 alerts subscriber for 4-LED task");
        let led_bq_alerts_sub = _bq76920_alerts_chan
            .subscriber()
            .expect("Allocate BQ76920 alerts subscriber for 4-LED task");
        let led_bal_cv_sub = balancing_cv_chan
            .subscriber()
            .expect("Allocate BalancingCv subscriber for 4-LED task");
        _spawner.spawn(
            leds4_task::leds_task(
                leds4_task::LedPins {
                    red: led_r,
                    yellow: led_y,
                    green: led_g,
                    blue: led_b,
                },
                led_bq_sub,
                led_sc_alerts_sub,
                led_bq_alerts_sub,
                led_bal_cv_sub,
            )
            .expect("led4 token"),
        );

        let bq76920_meas_sub = bq76920_meas_chan
            .subscriber()
            .expect("Allocate BQ76920 measurements subscriber");
        let i2c_dev_for_sc = I2cDevice::new(i2c_bus);
        let tmp75_i2c = I2cDevice::new(i2c_bus);
        let balancing_cv_sub = balancing_cv_chan
            .subscriber()
            .expect("Allocate BalancingCv subscriber for charger task");
        _spawner.spawn(
            sc8815_task::sc8815_task(sc8815_task::Sc8815TaskArgs {
                ce_ctl: ce,
                pstop_ctl: pstop,
                i2c_device: i2c_dev_for_sc,
                tmp75_i2c,
                address: sc8815_task::SC8815_DEFAULT_ADDRESS,
                sc8815_alerts_publisher: sc8815_alerts_pub,
                sc8815_measurements_publisher: sc8815_meas_pub,
                bq76920_measurements_subscriber: bq76920_meas_sub,
                balancing_cv_sub,
            })
            .expect("sc token"),
        );

        let i2c_dev_runtime = I2cDevice::new(i2c_bus);
        let sc8815_alerts_sub = _sc8815_alerts_chan
            .subscriber()
            .expect("Allocate SC8815 alerts subscriber for BQ task");
        _spawner.spawn(
            bq76920_task::bq76920_task(bq76920_task::Bq76920TaskArgs {
                i2c_bus: i2c_dev_runtime,
                address: bq_runtime_addr,
                sense_resistor_m_ohm: 3,
                ntc_params: None,
                bq76920_alerts_publisher: bq76920_alerts_pub,
                bq76920_measurements_publisher: bq76920_meas_pub,
                sc8815_alerts_subscriber: sc8815_alerts_sub,
                balancing_cv_publisher: balancing_cv_pub,
            })
            .expect("bq token"),
        );

        // Pack NTC + MCU temperature sampling (ADC1 + PA0..PA3 + PB12).
        let ntc_args = ntc_temp::NtcTempTaskArgs {
            adc: p.ADC1,
            ts45: p.PA0,
            ts34: p.PA1,
            ts23: p.PA2,
            ts12: p.PA3,
            ntc_vcc: p.PB12,
        };
        let ntc_token = ntc_temp::ntc_temp_task(ntc_args).expect("ntc-temp token");
        _spawner.spawn(ntc_token);

        _spawner.spawn(
            ship_button_task(ship_button, i2c_bus, bq_runtime_addr).expect("ship-button token"),
        );

        // 保留软件睡眠管理器（轻度 SLEEP 策略），由默认执行器 WFE 驱动。
        _spawner.spawn(sleep_manager::sleep_task().expect("sleep-mgr token"));

        // 电源静默策略改由 SC 路径直接更新至 failsafe，不再单独起调度任务

        // (Optional) EXTI for BQ ALERT can be enabled here if needed for additional wake sources.

        let sc_mirror_sub = sc8815_meas_chan
            .subscriber()
            .expect("Allocate SC8815 measurements subscriber for I2C mirror");
        _spawner.spawn(sc_meas_mirror_task(sc_mirror_sub).expect("sc8815-mirror token"));

        let bq_mirror_sub = bq76920_meas_chan
            .subscriber()
            .expect("Allocate BQ76920 measurements subscriber for I2C mirror");
        _spawner.spawn(bq_meas_mirror_task(bq_mirror_sub).expect("bq-mirror token"));

        let token = i2c_slave::task(i2c1_dev).expect("i2c1 task");
        _spawner.spawn(token);
        // (omit gs_mirror_task to reduce flash)

        // Idle task: periodically yield; low-power executor controls STOP entry.
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}
// 入口由 #[embassy_executor::main] 提供（线程模式 WFE）。
