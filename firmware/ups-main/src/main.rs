#![no_std]
#![no_main]

mod adin_temp;
mod fan_control;
mod io_expander;
mod tsens;

use defmt::{info, warn};
use embedded_hal::delay::DelayNs;
use esp_backtrace as _; // panic handler + backtrace/println
use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{
        channel, channel::ChannelIFace, timer, timer::TimerIFace, LSGlobalClkSource, Ledc, LowSpeed,
    },
    main,
    spi::{
        master::{Config as SpiConfig, Spi},
        Mode,
    },
    time::Rate,
};
use esp_println as _; // install UART logger + defmt bridge
use sc8815::{self, registers::constants::DEFAULT_ADDRESS as SC8815_ADDR};

// Populate the ESP-IDF App Descriptor so espflash can read metadata
esp_bootloader_esp_idf::esp_app_desc!();

// Provide millisecond timestamps for defmt logs
defmt::timestamp!("{=u64} ms", {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64
});

#[main]
fn main() -> ! {
    // Initialise chip peripherals
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    // === Restore board bring-up so other subsystems remain functional ===
    // Buttons (internal pull-up, active-low)
    let in_cfg = InputConfig::default().with_pull(Pull::Up);
    let _btn_center = Input::new(peripherals.GPIO0, in_cfg);
    let _btn_up = Input::new(peripherals.GPIO1, in_cfg);
    let _btn_right = Input::new(peripherals.GPIO2, in_cfg);
    let _btn_down = Input::new(peripherals.GPIO4, in_cfg);
    let _btn_left = Input::new(peripherals.GPIO5, in_cfg);

    // RESET# to TCA6408A
    let mut _reset_tca = Output::new(peripherals.GPIO6, Level::High, OutputConfig::default());
    _reset_tca.set_high();

    // INTn (open-drain, low-active) – configure as input with pull-up
    let _int_n = Input::new(
        peripherals.GPIO7,
        InputConfig::default().with_pull(Pull::Up),
    );

    // I2C0 @ 400kHz (SDA=GPIO8, SCL=GPIO9)
    let mut i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO9);

    // SPI for LCD (write-only): MOSI=11, SCLK=12, optional CS/DC/RST as GPIO
    let mut _dc = Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default());
    let mut _cs = Output::new(peripherals.GPIO13, Level::High, OutputConfig::default());
    let mut _rst = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    let _ = _rst.set_low();
    delay.delay_ms(10u32);
    let _ = _rst.set_high();

    let mut _spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO12)
    .with_mosi(peripherals.GPIO11);

    // USB2_PG (STAT) input – GPIO21 (pull-up)
    let _usb2_pg = Input::new(
        peripherals.GPIO21,
        InputConfig::default().with_pull(Pull::Up),
    );

    // LEDC PWM: FAN_PWM on GPIO40 (LowSpeed@25kHz), BUZZER on GPIO38 (LowSpeed@2kHz)
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut t_fan = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    t_fan
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(25),
        })
        .unwrap();

    let mut t_buz = ledc.timer::<LowSpeed>(timer::Number::Timer1);
    t_buz
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(2),
        })
        .unwrap();

    let mut fan_en = Output::new(peripherals.GPIO39, Level::Low, OutputConfig::default()); // FAN_EN (MTCK)
    let mut fan_pwm = ledc.channel(channel::Number::Channel0, peripherals.GPIO40);
    fan_pwm
        .configure(channel::config::Config {
            timer: &t_fan,
            duty_pct: 0,
            pin_config: channel::config::PinConfig::PushPull,
        })
        .unwrap();

    let mut buzzer = ledc.channel(channel::Number::Channel1, peripherals.GPIO38);
    buzzer
        .configure(channel::config::Config {
            timer: &t_buz,
            duty_pct: 0,
            pin_config: channel::config::PinConfig::PushPull,
        })
        .unwrap();

    info!("UPS main firmware booting…");
    info!("GPIO mappings: buttons center/up/right/down/left = 0/1/2/4/5");
    info!("I2C0 pins: SDA=GPIO8, SCL=GPIO9, INTn=GPIO7, USB2_PG=GPIO21");
    info!("SPI LCD pins: DC=GPIO10, MOSI=GPIO11, SCLK=GPIO12, CS=GPIO13, RST=GPIO14");
    info!("Fan control: EN=GPIO39, PWM=GPIO40; buzzer=GPIO38 (2kHz)");

    // Keep buzzer idle; fan controller will manage enable/duty automatically
    buzzer.set_duty(0).ok();
    fan_en.set_low();

    match io_expander::init(&mut i2c) {
        Ok(()) => info!("tca6408a: init ok (CE=high, PSTOP=high)"),
        Err(_) => warn!("tca6408a: init failed (safe state not verified)"),
    }

    match io_expander::read_in_pg(&mut i2c) {
        Ok(pg) => info!("tca6408a: IN_PG={}", if pg { "high" } else { "low" }),
        Err(_) => warn!("tca6408a: read IN_PG failed"),
    }

    i2c = log_sc8815_temperature(i2c, &mut delay);

    // === Temperature sensor init ===
    tsens::init(&mut delay);
    let delta_opt = tsens::read_delta_calibration();
    let delta_c = delta_opt.unwrap_or(0.0);

    info!("ups tsens bring-up: sampling once per second");
    if let Some(factory) = delta_opt {
        info!("TSENS calibration: delta={=f32}°C (from eFuse)", factory);
    } else {
        info!(
            "tsens calibration: efuse missing -> fallback delta={=f32}°C",
            delta_c
        );
    }
    delay.delay_ms(200u32);

    let vin_present = true; // VIN presence sensor pending, default to true per spec
    let mut controller = fan_control::FanController::new(fan_pwm, fan_en, delta_opt, vin_present);
    let mut adin_elapsed_ms: u32 = 0;

    loop {
        controller.tick(&mut delay);
        adin_elapsed_ms = adin_elapsed_ms.saturating_add(fan_control::SAMPLE_PERIOD_MS);
        if adin_elapsed_ms >= 1000 {
            adin_elapsed_ms -= 1000;
            i2c = log_sc8815_temperature(i2c, &mut delay);
        }
    }
}

fn log_sc8815_temperature<I2C, E>(mut i2c: I2C, delay: &mut Delay) -> I2C
where
    I2C: embedded_hal::i2c::I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut adin_mv_sample: Option<u16> = None;
    let mut ce_enabled = false;

    if let Err(_) = io_expander::set_sc_ce(&mut i2c, true) {
        warn!("sc8815: failed to pull CE low via TCA6408A");
    } else {
        ce_enabled = true;
    }

    delay.delay_ms(5u32);

    let mut sc = sc8815::SC8815::new(i2c, SC8815_ADDR);
    if ce_enabled {
        match sc.init() {
            Ok(()) => {
                if let Err(_) = sc.set_adc_conversion(true) {
                    warn!("sc8815: ADC start failed");
                } else {
                    delay.delay_ms(10u32);
                    match sc.get_adc_measurements() {
                        Ok(meas) => {
                            adin_mv_sample = Some(meas.adin_mv);
                        }
                        Err(_) => warn!("sc8815: ADC read failed"),
                    }
                    let _ = sc.set_adc_conversion(false);
                }
            }
            Err(_) => warn!("sc8815: init failed"),
        }
    }

    i2c = sc.release();

    if ce_enabled {
        if let Err(_) = io_expander::set_sc_ce(&mut i2c, false) {
            warn!("sc8815: failed to restore CE high");
        }
    }

    if let Some(adin_mv) = adin_mv_sample {
        match adin_temp::adin_mv_to_celsius(adin_mv) {
            Some(temp_c) => info!(
                "sc8815: ADIN temp ≈ {=f32} °C (from {=u16} mV)",
                temp_c, adin_mv
            ),
            None => warn!("sc8815: ADIN conversion out of range"),
        }
    }

    i2c
}
