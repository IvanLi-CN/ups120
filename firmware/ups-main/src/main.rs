#![no_std]
#![no_main]

mod adin_temp;
mod batt_est;
mod display;
mod fan_control;
mod io_expander;
mod tsens;
mod ui;

use core::cell::RefCell;

use defmt::{info, warn};
use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::I2c as _; // bring trait into scope for write_read without naming conflict
use embedded_hal_bus::i2c::RefCellDevice;
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
use io_expander::Tca6408a;
use sc8815::{self, registers::constants::DEFAULT_ADDRESS as SC8815_ADDR};
// STM32 smart-battery I2C slave (validation-only, single-shot)
const STM32_ADDR: u8 = 0x35;
const SB_SIG: [u8; 2] = [b'S', b'B'];
const SB_WINDOW_START: u8 = 0x08;
const SB_WINDOW_END: u8 = 0x0F;
const SB_TEMP_BASE: u8 = 0x14;
const SB_TEMP_DATA_BYTES: usize = 4;
const SB_TEMP_FRAME_BYTES: usize = SB_TEMP_DATA_BYTES * 2;
const TEST_A: u8 = 0x5A;
const TEST_B: u8 = 0xA5;

// Battery pack configuration (per project spec; do not probe at runtime)
const PACK_CELLS_S: u8 = 5; // 5S Li-ion (BQ76920 max 5S)
const SOC_EMPTY_VBAT_MV: u32 = 12_500; // Cutoff threshold (pack)
const SOC_FULL_VBAT_MV: u32 = 18_500; // Full threshold (pack)

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
    let boot_millis = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;

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

    // I2C0 @ 100kHz (SDA=GPIO8, SCL=GPIO9) — align with STM32 slave timing
    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO9);
    // Shared-bus wrapper for single-threaded access
    let i2c_bus = RefCell::new(i2c);

    // SPI for LCD (write-only): MOSI=11, SCLK=12, optional CS/DC/RST/BL as GPIO
    let mut _dc = Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default());
    let mut _cs = Output::new(peripherals.GPIO13, Level::High, OutputConfig::default());
    let mut _rst = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    // Backlight via LEDC will be configured below; keep BL pin as PWM (GPIO15)
    let _ = _rst.set_low();
    delay.delay_ms(10u32);
    let _ = _rst.set_high();

    // Before touching other devices on the bus, validate STM32 I2C once (aligned with fix/sb-comm-failure)
    delay.delay_ms(2u32);
    // Use shared-bus device for one-shot validation
    let mut i2c_dev_once = RefCellDevice::new(&i2c_bus);
    if let Err(_) = stm_one_shot_validate(&mut i2c_dev_once) {
        warn!("stm32: one-shot i2c validation failed");
    }

    let mut spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            // Use conservative 6 MHz per UI branch final; Mode 0 per main
            .with_frequency(Rate::from_mhz(6))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO12)
    .with_mosi(peripherals.GPIO11);
    // Reflect STM32 self-check completion on boot screen
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 15, "STM32 I2C Selfcheck");

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
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .unwrap();

    let mut buzzer = ledc.channel(channel::Number::Channel1, peripherals.GPIO38);
    buzzer
        .configure(channel::config::Config {
            timer: &t_buz,
            duty_pct: 0,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .unwrap();
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 25, "GPIO/LEDC PWM");

    // LCD Backlight on GPIO15 (LowSpeed@20kHz)
    let mut t_bl = ledc.timer::<LowSpeed>(timer::Number::Timer2);
    t_bl.configure(timer::config::Config {
        duty: timer::config::Duty::Duty8Bit,
        clock_source: timer::LSClockSource::APBClk,
        frequency: Rate::from_khz(20),
    })
    .unwrap();
    let mut backlight = ledc.channel(channel::Number::Channel2, peripherals.GPIO15);
    backlight
        .configure(channel::config::Config {
            timer: &t_bl,
            duty_pct: 85,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .unwrap();
    info!("LCD backlight set to 85% (20kHz)");

    info!("UPS main firmware booting…");
    info!("GPIO mappings: buttons center/up/right/down/left = 0/1/2/4/5");
    info!("I2C0 pins: SDA=GPIO8, SCL=GPIO9, INTn=GPIO7, USB2_PG=GPIO21");
    info!("SPI LCD pins: DC=GPIO10, MOSI=GPIO11, SCLK=GPIO12, CS=GPIO13, RST=GPIO14");
    info!("Fan control: EN=GPIO39, PWM=GPIO40; buzzer=GPIO38 (2kHz)");

    // Keep buzzer idle; fan controller will manage enable/duty automatically
    buzzer.set_duty(0).ok();
    fan_en.set_low();

    // Initialize LCD and draw boot UI once (non-blocking afterwards)
    info!("Initializing GC9D01 panel (160x50)...");
    if let Err(_) = display::init(&mut spi, &mut _cs, &mut _dc, &mut _rst, &mut delay) {
        warn!("gc9d01: init failed");
    } else {
        let _ = ui::boot_init_begin(&mut spi, &mut _cs, &mut _dc);
        let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 5, "SPI/LCD Ready");
        info!("Boot UI rendered");
    }

    // Global single-instance drivers
    let mut tca = Some(Tca6408a::new(RefCellDevice::new(&i2c_bus)));
    if let Some(t) = tca.as_mut() {
        match t.init() {
            Ok(()) => info!("tca6408a: init ok (CE=high, PSTOP=high)"),
            Err(_) => warn!("tca6408a: init failed (safe state not verified)"),
        }
        match t.read_in_pg() {
            Ok(pg) => info!("tca6408a: IN_PG={}", if pg { "high" } else { "low" }),
            Err(_) => warn!("tca6408a: read IN_PG failed"),
        }
    }
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 40, "TCA6408A");

    let mut sc = None;
    let mut sc_init_done: bool = false;
    let mut last_adin_temp_c: Option<f32> = None;
    log_sc8815_temperature(
        &i2c_bus,
        &mut delay,
        &mut tca,
        &mut sc,
        &mut sc_init_done,
        &mut last_adin_temp_c,
    );
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 60, "SC8815 ADC Read");

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
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 80, "TSENS Calibrated");

    let vin_present = true; // VIN presence sensor pending, default to true per spec
    let mut controller = fan_control::FanController::new(fan_pwm, fan_en, delta_opt, vin_present);
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 100, "Ready");
    let mut adin_elapsed_ms: u32 = 0;
    // UI: alternate SoC% and VBAT every ~2 seconds
    let mut soc_alt_counter: u8 = 0; // seconds within current phase
    let mut soc_alt_voltage: bool = false; // false => show %, true => show voltage
                                           // UI: rotate through available temperature sources every ~2 seconds
    let mut temp_alt_counter: u8 = 0;
    let mut temp_cycle_index: usize = 0;
    let mut sb_temps: Option<fan_control::SmartBatteryTemps> = None;

    loop {
        controller.tick(&mut delay, sb_temps);
        adin_elapsed_ms = adin_elapsed_ms.saturating_add(fan_control::SAMPLE_PERIOD_MS);
        if adin_elapsed_ms >= 1000 {
            adin_elapsed_ms -= 1000;
            // advance SoC display alternation (2-second cadence)
            soc_alt_counter = soc_alt_counter.saturating_add(1);
            if soc_alt_counter >= 2 {
                soc_alt_counter = 0;
                soc_alt_voltage = !soc_alt_voltage;
            }
            log_sc8815_temperature(
                &i2c_bus,
                &mut delay,
                &mut tca,
                &mut sc,
                &mut sc_init_done,
                &mut last_adin_temp_c,
            );
            let smart_batt = read_smart_battery_temperatures(&i2c_bus);
            sb_temps = smart_batt;

            let pack_temp = convert_temp_to_i16(sb_temps.and_then(|t| t.pack_c));
            let charger_temp = convert_temp_to_i16(sb_temps.and_then(|t| t.charger_c));
            let adin_temp = convert_temp_to_i16(last_adin_temp_c);

            let temp_sources = [
                (ui::TempSlot::Battery, pack_temp),
                (ui::TempSlot::Charger, charger_temp),
                (ui::TempSlot::Ups, adin_temp),
            ];

            let mut active_slots = [0usize; 3];
            let mut active_count = 0;
            for (idx, (_, value)) in temp_sources.iter().enumerate() {
                if value.is_some() {
                    active_slots[active_count] = idx;
                    active_count += 1;
                }
            }
            if active_count == 0 {
                active_slots[0] = 0;
                active_count = 1;
            }
            if temp_cycle_index >= active_count {
                temp_cycle_index = 0;
            }
            temp_alt_counter = temp_alt_counter.saturating_add(1);
            if temp_alt_counter >= 2 {
                temp_alt_counter = 0;
                if active_count > 0 {
                    temp_cycle_index = (temp_cycle_index + 1) % active_count;
                }
            }
            let current_slot_idx = active_slots[temp_cycle_index];
            let display_slot = temp_sources[current_slot_idx].0;

            // Estimate SoC from VBAT (do not probe CELLS_PRESENT at runtime)
            let vbat_mv = read_smart_battery_vbat_mv(&i2c_bus);
            let soc_pct = vbat_mv.map(|v| estimate_soc_from_vbat(v)).unwrap_or(0);
            let now_millis = esp_hal::time::Instant::now()
                .duration_since_epoch()
                .as_millis() as u64;
            let uptime_secs = now_millis
                .saturating_sub(boot_millis)
                .saturating_div(1000)
                .min(u64::from(u32::MAX)) as u32;

            // Build a minimal dashboard model (real temps + SoC; other fields placeholder)
            let model = ui::DashboardData {
                mode: ui::Mode::Standby,
                soc_pct: soc_pct,
                vbat_mv: vbat_mv,
                soc_display: if soc_alt_voltage {
                    ui::SocDisplay::Voltage
                } else {
                    ui::SocDisplay::Percent
                },
                in_v_mv: 0,
                in_a_ma: 0,
                in_w_mw: 0,
                chg_w_mw: 0,
                out_v_mv: 0,
                out_a_ma: 0,
                out_w_mw: 0,
                bat_temp_c: pack_temp,
                charger_temp_c: charger_temp,
                ups_temp_c: adin_temp,
                fan_pct: 0,
                uptime_secs,
                temp_slot: display_slot,
            };
            let _ = ui::render_dashboard_once(&mut spi, &mut _cs, &mut _dc, &model);
        }
    }
}

fn log_sc8815_temperature<'a, I2C, E>(
    i2c_bus: &'a RefCell<I2C>,
    delay: &mut Delay,
    tca: &mut Option<Tca6408a<RefCellDevice<'a, I2C>>>,
    sc: &mut Option<sc8815::SC8815<RefCellDevice<'a, I2C>>>,
    sc_init_done: &mut bool,
    last_adin_temp_c: &mut Option<f32>,
) where
    I2C: embedded_hal::i2c::I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut adin_mv_sample: Option<u16> = None;
    let mut ce_enabled = false;

    if let Some(t) = tca.as_mut() {
        if let Err(_) = t.set_sc_ce(true) {
            warn!("sc8815: failed to pull CE low via TCA6408A");
        } else {
            ce_enabled = true;
        }
    }

    delay.delay_ms(5u32);

    if ce_enabled {
        // Ensure instance exists
        if sc.is_none() {
            let dev = RefCellDevice::new(i2c_bus);
            let mut drv = sc8815::SC8815::new(dev, SC8815_ADDR);
            if !*sc_init_done {
                match drv.init() {
                    Ok(()) => *sc_init_done = true,
                    Err(_) => warn!("sc8815: init failed"),
                }
            }
            *sc = Some(drv);
        }

        if let Some(d) = sc.as_mut() {
            if let Err(_) = d.set_adc_conversion(true) {
                warn!("sc8815: ADC start failed");
            } else {
                delay.delay_ms(10u32);
                if let Ok(meas) = d.get_adc_measurements() {
                    adin_mv_sample = Some(meas.adin_mv);
                }
                let _ = d.set_adc_conversion(false);
            }
        }
    }

    if let Some(t) = tca.as_mut() {
        if ce_enabled {
            if let Err(_) = t.set_sc_ce(false) {
                warn!("sc8815: failed to restore CE high");
            }
        }
    }

    if let Some(adin_mv) = adin_mv_sample {
        if let Some(temp_c) = adin_temp::adin_mv_to_celsius(adin_mv) {
            *last_adin_temp_c = Some(temp_c);
        }
    }
}

fn read_smart_battery_temperatures<'a, I2C, E>(
    i2c_bus: &'a RefCell<I2C>,
) -> Option<fan_control::SmartBatteryTemps>
where
    I2C: embedded_hal::i2c::I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    // STM32 smart-battery slave currently exposes raw registers without CRC interleaving.
    // Use pointer write then read 4 bytes: [TPACK_L, TPACK_H, TCHG_L, TCHG_H] (i16 LE, 0.01°C; i16::MIN = invalid).
    let mut data = [0u8; 4];
    let mut dev = RefCellDevice::new(i2c_bus);
    match dev.write_read(STM32_ADDR, &[SB_TEMP_BASE], &mut data) {
        Ok(()) => {
            let pack = i16::from_le_bytes([data[0], data[1]]);
            let charger = i16::from_le_bytes([data[2], data[3]]);
            let pack_c = temp_from_centi(pack);
            let charger_c = temp_from_centi(charger);
            let temps = fan_control::SmartBatteryTemps::new(pack_c, charger_c);
            info!(
                "smart-battery temps => pack={=f32}°C charger={=f32}°C hottest={=f32}°C",
                pack_c.unwrap_or(f32::NAN),
                charger_c.unwrap_or(f32::NAN),
                temps.highest().unwrap_or(f32::NAN)
            );
            Some(temps)
        }
        Err(_) => {
            warn!("stm32: temp read failed");
            None
        }
    }
}

fn read_smart_battery_vbat_mv<'a, I2C, E>(i2c_bus: &'a RefCell<I2C>) -> Option<u32>
where
    I2C: embedded_hal::i2c::I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = RefCellDevice::new(i2c_bus);
    let mut vbuf = [0u8; 2];
    if dev.write_read(STM32_ADDR, &[0x10], &mut vbuf).is_ok() {
        let v = u16::from_le_bytes(vbuf) as u32;
        Some(v)
    } else {
        None
    }
}

fn estimate_soc_from_vbat(vbat_mv: u32) -> u8 {
    if vbat_mv <= SOC_EMPTY_VBAT_MV {
        return 0;
    }
    if vbat_mv >= SOC_FULL_VBAT_MV {
        return 100;
    }
    let span = SOC_FULL_VBAT_MV - SOC_EMPTY_VBAT_MV;
    let val = ((vbat_mv - SOC_EMPTY_VBAT_MV) * 100) / span;
    val as u8
}

fn round_temp_to_i16(temp: f32) -> i16 {
    if temp.is_sign_negative() {
        (temp - 0.5) as i16
    } else {
        (temp + 0.5) as i16
    }
}

fn convert_temp_to_i16(temp: Option<f32>) -> Option<i16> {
    temp.filter(|t| t.is_finite()).map(|t| round_temp_to_i16(t))
}

fn temp_from_centi(raw: i16) -> Option<f32> {
    if raw == i16::MIN {
        None
    } else {
        Some(raw as f32 / 100.0)
    }
}

// (reserved) SMBus CRC8 helper left here if smart-battery read switches to interleaved CRC in future
#[allow(dead_code)]
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn stm_one_shot_validate<I2C, E>(i2c: &mut I2C) -> Result<(), ()>
where
    I2C: embedded_hal::i2c::I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    // Write window byte at 0x08
    let set_a = [SB_WINDOW_START, TEST_A];
    i2c.write(STM32_ADDR, &set_a).map_err(|_| ())?;

    // Read 16 bytes from 0x00, confirm signature and window value
    let mut buf = [0u8; 16];
    i2c.write_read(STM32_ADDR, &[0x00], &mut buf)
        .map_err(|_| ())?;
    if buf[0] != SB_SIG[0] || buf[1] != SB_SIG[1] {
        warn!("stm32: signature mismatch {:02x} {:02x}", buf[0], buf[1]);
        return Err(());
    }
    if buf[SB_WINDOW_START as usize] != TEST_A {
        warn!(
            "stm32: window mismatch at 0x08 -> {:02x}",
            buf[SB_WINDOW_START as usize]
        );
        return Err(());
    }
    info!("stm32: dump[0..16]={=[u8]:02x}", &buf[..]);

    // Wraparound check: write 0x0E two bytes, then read back 4
    let set_tail = [SB_WINDOW_END - 1, TEST_A, TEST_B];
    i2c.write(STM32_ADDR, &set_tail).map_err(|_| ())?;
    let mut tail = [0u8; 4];
    i2c.write_read(STM32_ADDR, &[SB_WINDOW_END - 1], &mut tail)
        .map_err(|_| ())?;
    info!("stm32: tail={=[u8]:02x}", &tail[..]);
    if !(tail[0] == TEST_A && tail[1] == TEST_B) {
        return Err(());
    }

    Ok(())
}
