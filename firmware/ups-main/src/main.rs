#![no_std]
#![no_main]

use core::fmt::Write as _;

use defmt::*;
use esp_backtrace as _; // panic handler + backtrace/println
use esp_hal::{
    clock::ClockControl,
    delay::Delay,
    peripherals::Peripherals,
    prelude::*,
    timer::TimerGroup,
    gpio::{GpioPin, Input, PullUp, Output, PushPull, AnyPin},
    i2c::I2C,
    spi::{Spi, SpiMode, SpiDeviceDriver, FullDuplexMode},
    ledc::{channel, timer::{self, Timer as LedcTimer}, Ledc},
    uart::Uart,
};
use esp_println::println;

#[esp_hal::entry]
fn main() -> ! {
    // Peripherals, clocks, systimer
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();
    let mut delay = Delay::new(&clocks);

    // IO
    let io = esp_hal::io::Io::new(peripherals.GPIO, peripherals.IO_MUX);

    // Buttons (internal pull-up, active-low)
    let _btn_center: Input<PullUp> = io.pins.gpio0.into_pull_up_input();
    let _btn_up:     Input<PullUp> = io.pins.gpio1.into_pull_up_input();
    let _btn_right:  Input<PullUp> = io.pins.gpio2.into_pull_up_input();
    let _btn_down:   Input<PullUp> = io.pins.gpio4.into_pull_up_input();
    let _btn_left:   Input<PullUp> = io.pins.gpio5.into_pull_up_input();

    // RESET# to TCA6408A (suggest internal pull-up)
    let mut _reset_tca: Output<PushPull> = io.pins.gpio6.into_push_pull_output();
    _reset_tca.set_high();

    // INT (open-drain, low-active) – configure as input with pull-up
    let _int_n: Input<PullUp> = io.pins.gpio7.into_pull_up_input();

    // I2C (SDA=GPIO8, SCL=GPIO9) at 400 kHz typical
    let sda = io.pins.gpio8;
    let scl = io.pins.gpio9;
    let mut i2c = I2C::new(peripherals.I2C0, sda, scl, 400.kHz(), &clocks);

    // SPI for LCD (DC=10, MOSI=11, SCLK=12, CS=13, RES=14)
    let spi = Spi::new(
        peripherals.SPI2,
        40.MHz(),
        SpiMode::Mode0,
        &clocks,
    );
    let sclk = io.pins.gpio12;
    let mosi = io.pins.gpio11;
    let miso = Option::<esp_hal::gpio::GpioPin<_, _>>::None; // display write-only
    let dc   = io.pins.gpio10.into_push_pull_output();
    let cs   = io.pins.gpio13.into_push_pull_output();
    let rst  = io.pins.gpio14.into_push_pull_output();
    let mut spi = spi.with_pins(sclk, mosi, miso);

    // USB2_PG (STAT) input – GPIO21, open-drain low-active
    let _usb2_pg: Input<PullUp> = io.pins.gpio21.into_pull_up_input();

    // LEDC PWM: FAN_PWM on GPIO40 (MTDO), BUZZER on GPIO38
    let ledc = Ledc::new(peripherals.LEDC, &clocks);
    let mut ledc_t0 = LedcTimer::new(ledc.timer0, &clocks, timer::config::Config::default().frequency(25.kHz()));
    let mut ledc_t1 = LedcTimer::new(ledc.timer1, &clocks, timer::config::Config::default().frequency(2.kHz()));

    let fan_en = io.pins.gpio39.into_push_pull_output(); // FAN_EN (MTCK)
    let fan_pwm_pin = io.pins.gpio40; // FAN_PWM (MTDO)
    let buzzer_pin  = io.pins.gpio38; // BUZZER

    let mut fan_pwm = channel::Channel::new(ledc.channel0, fan_pwm_pin);
    fan_pwm.set_timer(&mut ledc_t0);
    fan_pwm.set_duty(0);

    let mut buzzer = channel::Channel::new(ledc.channel1, buzzer_pin);
    buzzer.set_timer(&mut ledc_t1);
    buzzer.set_duty(0);

    println!("UPS main firmware booting…");
    println!("Buttons: center/up/right/down/left on GPIO0/1/2/4/5");
    println!("I2C0 on GPIO8/9; INTn on GPIO7; USB2_PG on GPIO21");
    println!("SPI LCD: DC10 MOSI11 SCLK12 CS13 RST14");
    println!("Fan: EN39 PWM40; Buzzer: 38 (2kHz)");

    // Bring-up beep: 200 ms tone at 2 kHz, then stop
    buzzer.set_duty(buzzer.get_max_duty() / 8);
    delay.delay_millis(200);
    buzzer.set_duty(0);

    // Enable fan at low PWM as a smoke-test
    let mut fan_en = fan_en;
    fan_en.set_high();
    fan_pwm.set_duty(fan_pwm.get_max_duty() / 10);

    // Probe I2C devices (TCA6408A at 0x20 expected)
    let _ = i2c.write(0x20u8, &[]); // ignore errors during smoke-test

    loop {
        // Simple heartbeat delay to keep PWM running
        delay.delay_millis(1000);
    }
}

