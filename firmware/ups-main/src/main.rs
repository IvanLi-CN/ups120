#![no_std]
#![no_main]

mod tsens;

use defmt::info;
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
use esp_println::{self as _, println}; // install UART logger + println! macro + defmt bridge

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

    println!("UPS main firmware booting…");
    println!("Buttons: center/up/right/down/left on GPIO0/1/2/4/5");
    println!("I2C0 on GPIO8/9; INTn on GPIO7; USB2_PG on GPIO21");
    println!("SPI LCD: DC10 MOSI11 SCLK12 CS13 RST14");
    println!("Fan: EN39 PWM40; Buzzer: 38 (2kHz)");

    // Bring-up beep: ~12% duty, 200 ms @ 2 kHz
    buzzer.set_duty(12).ok();
    delay.delay_ms(200u32);
    buzzer.set_duty(0).ok();

    // Enable fan at low PWM as a smoke-test (~10%)
    fan_en.set_high();
    fan_pwm.set_duty(10).ok();

    // Probe I2C device (TCA6408A 0x20) — ignore errors during smoke-test
    let _ = i2c.write(0x20u8, &[]);

    // === Temperature sensor init ===
    tsens::init(&mut delay);
    let delta_opt = tsens::read_delta_calibration();
    let delta_c = delta_opt.unwrap_or(0.0);

    info!("ups tsens bring-up: sampling once per second");
    if let Some(factory) = delta_opt {
        info!("tsens calibration: delta={=f32}°C", factory);
    } else {
        info!(
            "tsens calibration: efuse missing -> fallback delta={=f32}°C",
            delta_c
        );
    }
    delay.delay_ms(200u32);

    loop {
        let reading = tsens::read_celsius(&mut delay);
        let corrected = reading.base_celsius - delta_c;
        info!(
            "tsens sample: temp={=f32}°C base={=f32}°C raw={=u8} dac=0x{=u8:X}",
            corrected, reading.base_celsius, reading.raw, reading.dac
        );
        delay.delay_ms(1000u32);
    }
}
