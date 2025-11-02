#![no_std]
#![no_main]

use embedded_hal::delay::DelayNs;
use esp_backtrace as _; // panic handler
use esp_hal::{
    delay::Delay,
    i2c::master::{Config as I2cConfig, I2c},
    main,
    time::Rate,
};
use esp_println::println;

const STM32_ADDR: u8 = 0x35;
const SIG_BYTES: [u8; 2] = [b'S', b'B'];
const SIG_ADDR: usize = 0x00;
const WINDOW_START: u8 = 0x08;
const WINDOW_END: u8 = 0x0F;
const WINDOW_VALUE_IDX: usize = WINDOW_START as usize;
const TEST_VALUE_A: u8 = 0x5A;
const TEST_VALUE_B: u8 = 0xA5;

// Populate the ESP-IDF App Descriptor so espflash can read metadata
esp_bootloader_esp_idf::esp_app_desc!();

#[derive(Debug)]
enum TestError {
    Bus(esp_hal::i2c::master::Error),
    Check(&'static str),
}

impl From<esp_hal::i2c::master::Error> for TestError {
    fn from(err: esp_hal::i2c::master::Error) -> Self {
        TestError::Bus(err)
    }
}

type TestResult = Result<(), TestError>;

fn write_and_verify_signature<B>(i2c: &mut B) -> TestResult
where
    B: embedded_hal::i2c::I2c<Error = esp_hal::i2c::master::Error>,
{
    i2c.write(STM32_ADDR, &[WINDOW_START, TEST_VALUE_A])?;
    let mut buf = [0u8; 16];
    i2c.write_read(STM32_ADDR, &[0x00], &mut buf)?;
    println!("test1 raw: {:02x?}", buf);
    if buf[SIG_ADDR] != SIG_BYTES[0] || buf[SIG_ADDR + 1] != SIG_BYTES[1] {
        return Err(TestError::Check("signature mismatch"));
    }
    if buf[WINDOW_VALUE_IDX] != TEST_VALUE_A {
        return Err(TestError::Check("window value mismatch"));
    }
    println!("test1 dump: {:02x?}", buf);
    Ok(())
}

fn wraparound_write_read<B>(i2c: &mut B) -> TestResult
where
    B: embedded_hal::i2c::I2c<Error = esp_hal::i2c::master::Error>,
{
    i2c.write(STM32_ADDR, &[WINDOW_END - 1, TEST_VALUE_A, TEST_VALUE_B])?;
    let mut buf = [0u8; 4];
    i2c.write_read(STM32_ADDR, &[WINDOW_END - 1], &mut buf)?;
    if buf[0] != TEST_VALUE_A || buf[1] != TEST_VALUE_B {
        return Err(TestError::Check("wrap write failed"));
    }
    println!("test2 window tail: {:02x?}", buf);
    Ok(())
}

fn implicit_pointer_read<B>(i2c: &mut B) -> TestResult
where
    B: embedded_hal::i2c::I2c<Error = esp_hal::i2c::master::Error>,
{
    i2c.write(STM32_ADDR, &[0x00])?;
    let mut buf = [0u8; 3];
    i2c.read(STM32_ADDR, &mut buf)?;
    if buf[0..2] != SIG_BYTES {
        return Err(TestError::Check("implicit pointer signature mismatch"));
    }
    println!("test3 implicit read: {:02x?}", buf);
    Ok(())
}

fn nack_detection<B>(i2c: &mut B) -> TestResult
where
    B: embedded_hal::i2c::I2c<Error = esp_hal::i2c::master::Error>,
{
    match i2c.write(STM32_ADDR ^ 0x02, &[0x00]) {
        Err(err) => {
            println!("test4 expected nack: {:?}", err);
            Ok(())
        }
        Ok(_) => Err(TestError::Check("unexpected ack from invalid address")),
    }
}

fn report_error(test: &str, err: TestError) {
    match err {
        TestError::Bus(e) => println!("{test}: bus error {:?}", e),
        TestError::Check(reason) => println!("{test}: check failed ({reason})"),
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    let config = I2cConfig::default().with_frequency(Rate::from_khz(100));
    let i2c = I2c::new(peripherals.I2C0, config).expect("init I2C0");
    let mut i2c = i2c.with_sda(peripherals.GPIO8).with_scl(peripherals.GPIO9);

    println!(
        "esp32: i2c validation → stm32 addr=0x{STM32_ADDR:02x}, build_ts={}",
        env!("ESP_BUILD_TS")
    );

    let mut failed = false;

    println!("running signature-window-write");
    if let Err(err) = write_and_verify_signature(&mut i2c) {
        report_error("signature-window-write", err);
        failed = true;
    }

    if !failed {
        println!("running wraparound-write");
        if let Err(err) = wraparound_write_read(&mut i2c) {
            report_error("wraparound-write", err);
            failed = true;
        }
    }

    if !failed {
        println!("running implicit-pointer-read");
        if let Err(err) = implicit_pointer_read(&mut i2c) {
            report_error("implicit-pointer-read", err);
            failed = true;
        }
    }

    if !failed {
        println!("running nack-detection");
        if let Err(err) = nack_detection(&mut i2c) {
            report_error("nack-detection", err);
            failed = true;
        }
    }

    if failed {
        println!("scenario sequence terminated early");
    } else {
        println!("all scenarios completed successfully");
    }

    drop(i2c);

    loop {
        delay.delay_ms(1000u32);
    }
}
