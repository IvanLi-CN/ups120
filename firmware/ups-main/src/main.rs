#![no_std]
#![no_main]

mod tsens;

use defmt::info;
use embedded_hal::delay::DelayNs;
use esp_backtrace as _; // panic handler + backtrace/println
use esp_hal::{delay::Delay, main};
use esp_println as _; // install UART logger + defmt bridge

const DIE_TO_AMBIENT_OFFSET_C: f32 = 5.0;

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
    // Initialise chip peripherals; we only keep TSENS-powered blocks enabled.
    let _peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

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
        let corrected = reading.base_celsius - delta_c + DIE_TO_AMBIENT_OFFSET_C;
        info!(
            "tsens sample: temp={=f32}°C base={=f32}°C raw={=u8} dac=0x{=u8:X}",
            corrected, reading.base_celsius, reading.raw, reading.dac
        );
        delay.delay_ms(1000u32);
    }
}
