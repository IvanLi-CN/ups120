#![no_std]
#![no_main]

mod adin_temp;
mod batt_est;
mod button_input;
mod display;
mod fan_control;
mod io_expander;
mod tsens;
mod ui;

use button_input::{ButtonConfig, ButtonState};
use defmt::{debug, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{Channel, Receiver, Sender},
    mutex::Mutex,
};
use embassy_time::{Duration, Timer};
use embedded_hal::delay::DelayNs;
use embedded_hal_async::i2c::I2c as AsyncI2c;
use esp_backtrace as _; // panic handler + backtrace/println
use esp_hal::{
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    dma_buffers,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{
        channel, channel::ChannelIFace, timer, timer::TimerIFace, LSGlobalClkSource, Ledc, LowSpeed,
    },
    spi::{
        master::{Config as SpiConfig, Spi},
        Mode,
    },
    time::Rate,
    timer::timg::TimerGroup,
    Async,
};
use esp_println as _; // install UART logger + defmt bridge
use io_expander::Tca6408a;
use sc8815::{self, registers::constants::DEFAULT_ADDRESS as SC8815_ADDR};
use static_cell::StaticCell;
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
const SB_REG_CHG_CONFIG: u8 = 0x31;
const SB_REG_CHG_PAUSE_CAUSE: u8 = 0x32;
const SB_REG_STATE_FLAGS: u8 = 0x20;
const SB_STATE_FLAG_AC_PRESENT: u16 = 0x0001;
// Mirror of STM32 smart-battery state_bits::BALANCING; used for UI overlay.
const SB_STATE_FLAG_BALANCING: u16 = 1 << 5;
const SB_CHG_STATUS_BALANCING: u8 = 1 << 5;
const SB_CFG_BIT_AUTO: u8 = 1 << 0;
const SB_CFG_BIT_MANUAL: u8 = 1 << 1;
const SB_CFG_SPEED_SHIFT: u8 = 2;

// Battery pack configuration (per project spec; do not probe at runtime)
const PACK_CELLS_S: u8 = 5; // 5S Li-ion (BQ76920 max 5S)
const SOC_EMPTY_VBAT_MV: u32 = 12_500; // Cutoff threshold (pack)
const SOC_FULL_VBAT_MV: u32 = 18_500; // Full threshold (pack)
const CHARGE_START_VBAT_MV: u32 = 17_000;
const CHARGE_STOP_VBAT_MV: u32 = SOC_FULL_VBAT_MV;
/// AC 适配器恢复后，IN_PG 连续为 High 至少 10 s 才允许重新开始充电。
const AC_STABLE_MS: u64 = 10_000;
const TEMP_PAUSE_C: f32 = 40.0;
const TEMP_RESUME_C: f32 = 35.0;
const SB_WRITE_RETRY_ATTEMPTS: u8 = 3;
const SB_WRITE_RETRY_DELAY_MS: u32 = 5;
const SB_CFG_VERIFY_INTERVAL_MS: u64 = 1_000;
const SB_STATE_POLL_INTERVAL_MS: u64 = 10_000;

type I2cBusMutex = Mutex<NoopRawMutex, I2c<'static, Async>>;
type SharedI2cDevice<'a> = I2cDevice<'a, NoopRawMutex, I2c<'static, Async>>;
static I2C0_BUS: StaticCell<I2cBusMutex> = StaticCell::new();

const UI_EVENT_CAPACITY: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiScreen {
    Dashboard,
    BattDetail,
}

#[derive(Clone, Copy)]
enum UiEvent {
    SwitchToDashboard,
    SwitchToBattDetail,
}

type UiEventSender = Sender<'static, NoopRawMutex, UiEvent, UI_EVENT_CAPACITY>;
type UiEventReceiver = Receiver<'static, NoopRawMutex, UiEvent, UI_EVENT_CAPACITY>;

static UI_EVENT_CHANNEL: StaticCell<Channel<NoopRawMutex, UiEvent, UI_EVENT_CAPACITY>> =
    StaticCell::new();

fn compose_sb_charge_config(auto: bool, manual: bool, speed_tier: u8) -> u8 {
    let mut value = (speed_tier & 0x03) << SB_CFG_SPEED_SHIFT;
    if auto {
        value |= SB_CFG_BIT_AUTO;
    }
    if manual {
        value |= SB_CFG_BIT_MANUAL;
    }
    value
}

// Populate the ESP-IDF App Descriptor so espflash can read metadata
esp_bootloader_esp_idf::esp_app_desc!();

// Provide millisecond timestamps for defmt logs
defmt::timestamp!("{=u64} ms", {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64
});

#[embassy_executor::task]
async fn button_task(
    btn_center: Input<'static>,
    btn_up: Input<'static>,
    btn_right: Input<'static>,
    btn_down: Input<'static>,
    btn_left: Input<'static>,
    ui_tx: UiEventSender,
    boot_millis: u64,
) {
    // Initialize per-button state machines for debounced/gesture-aware logging.
    let center_initial = btn_center.is_low();
    let up_initial = btn_up.is_low();
    let right_initial = btn_right.is_low();
    let down_initial = btn_down.is_low();
    let left_initial = btn_left.is_low();

    // Default configs: long + double enabled for all keys.
    let cfg_center = ButtonConfig::new("center");
    let cfg_up = ButtonConfig::new("up");
    let cfg_right = ButtonConfig::new("right");
    let cfg_down = ButtonConfig::new("down");
    let cfg_left = ButtonConfig::new("left");

    let mut btn_center_state = ButtonState::new(cfg_center, center_initial, boot_millis);
    let mut btn_up_state = ButtonState::new(cfg_up, up_initial, boot_millis);
    let mut btn_right_state = ButtonState::new(cfg_right, right_initial, boot_millis);
    let mut btn_down_state = ButtonState::new(cfg_down, down_initial, boot_millis);
    let mut btn_left_state = ButtonState::new(cfg_left, left_initial, boot_millis);

    let mut prev_down_pressed = btn_down_state.is_pressed();
    let mut prev_up_pressed = btn_up_state.is_pressed();

    info!(
        "Initial button state: center={} up={} right={} down={} left={}",
        center_initial, up_initial, right_initial, down_initial, left_initial
    );

    loop {
        let now_ms = esp_hal::time::Instant::now()
            .duration_since_epoch()
            .as_millis() as u64;

        btn_center_state.update(now_ms, btn_center.is_low());
        btn_up_state.update(now_ms, btn_up.is_low());
        btn_right_state.update(now_ms, btn_right.is_low());
        btn_down_state.update(now_ms, btn_down.is_low());
        btn_left_state.update(now_ms, btn_left.is_low());

        // Derive simple edge-triggered navigation events from debounced states.
        let down_pressed = btn_down_state.is_pressed();
        let up_pressed = btn_up_state.is_pressed();

        if down_pressed && !prev_down_pressed {
            let _ = ui_tx.try_send(UiEvent::SwitchToBattDetail);
        }
        if up_pressed && !prev_up_pressed {
            let _ = ui_tx.try_send(UiEvent::SwitchToDashboard);
        }

        prev_down_pressed = down_pressed;
        prev_up_pressed = up_pressed;

        Timer::after(Duration::from_millis(fan_control::SAMPLE_PERIOD_MS.into())).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Initialise chip peripherals
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    let mut delay = Delay::new();
    let boot_millis = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;

    // === Restore board bring-up so other subsystems remain functional ===
    // Buttons (internal pull-up, active-low)
    let in_cfg = InputConfig::default().with_pull(Pull::Up);
    let btn_center = Input::new(peripherals.GPIO0, in_cfg);
    let btn_up = Input::new(peripherals.GPIO1, in_cfg);
    let btn_right = Input::new(peripherals.GPIO2, in_cfg);
    let btn_down = Input::new(peripherals.GPIO4, in_cfg);
    let btn_left = Input::new(peripherals.GPIO5, in_cfg);

    let ui_channel = UI_EVENT_CHANNEL.init(Channel::new());
    let ui_event_tx: UiEventSender = ui_channel.sender();
    let ui_event_rx: UiEventReceiver = ui_channel.receiver();

    // UI navigation state: dashboard vs battery detail, plus blink phase.
    let mut ui_screen = UiScreen::Dashboard;
    let mut blink_on = true;
    let mut blink_elapsed_ms: u32 = 0;

    // Spawn asynchronous button scanner task.
    let _ = spawner.spawn(button_task(
        btn_center,
        btn_up,
        btn_right,
        btn_down,
        btn_left,
        ui_event_tx,
        boot_millis,
    ));

    // RESET# to TCA6408A
    let mut _reset_tca = Output::new(peripherals.GPIO6, Level::High, OutputConfig::default());
    _reset_tca.set_high();

    // INTn (open-drain, low-active) – configure as input with pull-up
    let _int_n = Input::new(
        peripherals.GPIO7,
        InputConfig::default().with_pull(Pull::Up),
    );

    // I2C0 @ 100kHz (SDA=GPIO8, SCL=GPIO9) — align with STM32 slave timing.
    // Configure in async mode and expose via an Embassy shared-bus mutex so that
    // all peripherals (STM32 smart-battery, TCA6408A, SC8815) use real async I2C
    // transactions driven by the hardware peripheral.
    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO9)
    .into_async();
    let i2c_bus = I2C0_BUS.init(Mutex::new(i2c));

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
    // Use async shared-bus device for one-shot validation
    let mut i2c_dev_once = I2cDevice::new(i2c_bus);
    if let Err(_) = stm_one_shot_validate(&mut i2c_dev_once).await {
        warn!("stm32: one-shot i2c validation failed");
    }

    const SB_AUTO_ENABLED: bool = false;
    let sb_speed_tier: u8 = 0x01; // ≈0.8A
    let mut sb_manual_enable = false;
    let mut sb_config_value =
        compose_sb_charge_config(SB_AUTO_ENABLED, sb_manual_enable, sb_speed_tier);
    let mut sb_cfg_last_verify_ms = 0u64;
    let mut sb_temp_pause_active = false;
    let mut sb_last_vbat_mv: Option<u32> = None;
    let mut sb_last_state_poll_ms = 0u64;
    let mut last_state_flags: Option<u16> = None;
    let mut last_cells_mv: [Option<u16>; 5] = [None; 5];
    {
        let mut sb_i2c = I2cDevice::new(i2c_bus);
        match write_smart_battery_reg_retry(&mut sb_i2c, SB_REG_CHG_CONFIG, sb_config_value).await {
            Ok(()) => info!("smart-battery: config set (manual ctl, tier=0.8A)"),
            Err(()) => warn!("smart-battery: failed to apply charge config"),
        }
    }

    // SPI for LCD (write-only): MOSI=11, SCLK=12, optional CS/DC/RST/BL as GPIO.
    // Configure with DMA and async mode so higher-level code can migrate to embedded-hal-async
    // without changing the hardware setup again.
    let (rx_buf, rx_desc, tx_buf, tx_desc) = dma_buffers!(4096);
    let dma_rx = DmaRxBuf::new(rx_desc, rx_buf).unwrap();
    let dma_tx = DmaTxBuf::new(tx_desc, tx_buf).unwrap();

    let mut spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            // Use conservative 6 MHz per UI branch final; Mode 0 per main
            .with_frequency(Rate::from_mhz(6))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO12)
    .with_mosi(peripherals.GPIO11)
    .with_dma(peripherals.DMA_CH0)
    .with_buffers(dma_rx, dma_tx)
    .into_async();
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
    if let Err(_) = display::init_async(&mut spi, &mut _cs, &mut _dc, &mut _rst).await {
        warn!("gc9d01: init failed");
    } else {
        let _ = ui::boot_init_begin(&mut spi, &mut _cs, &mut _dc);
        let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 5, "SPI/LCD Ready");
        info!("Boot UI rendered");
    }

    // Global single-instance drivers (async I2C devices on the shared bus)
    let mut tca = Some(Tca6408a::new(I2cDevice::new(i2c_bus)));
    let mut vin_present = true;
    // Track last time IN_PG changed so we can derive a “stable for AC_STABLE_MS” window.
    let mut vin_state_last_change_ms: u64 = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;
    let mut last_in_pg_logged: Option<bool> = None;
    let mut in_pg_read_failed = false;
    let mut charge_skip_adapter_logged = false;
    if let Some(t) = tca.as_mut() {
        match t.init().await {
            Ok(()) => info!("tca6408a: init ok (CE=high, PSTOP=high)"),
            Err(_) => warn!("tca6408a: init failed (safe state not verified)"),
        }
        match t.read_in_pg().await {
            Ok(pg) => {
                vin_present = pg;
                vin_state_last_change_ms = esp_hal::time::Instant::now()
                    .duration_since_epoch()
                    .as_millis() as u64;
                last_in_pg_logged = Some(pg);
                info!("tca6408a: IN_PG={}", if pg { "high" } else { "low" });
            }
            Err(_) => {
                warn!("tca6408a: read IN_PG failed");
                in_pg_read_failed = true;
            }
        }
    }
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 40, "TCA6408A");

    let mut sc: Option<sc8815::SC8815<SharedI2cDevice<'static>>> = None;
    let mut sc_init_done: bool = false;
    let mut last_adin_temp_c: Option<f32> = None;
    log_sc8815_temperature(
        i2c_bus,
        &mut tca,
        &mut sc,
        &mut sc_init_done,
        &mut last_adin_temp_c,
    )
    .await;
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

        // Maintain the legacy 20 ms control-loop cadence using an async
        // timer so that other Embassy tasks (e.g. button_task) can run
        // between iterations instead of being blocked by a busy delay.
        Timer::after(Duration::from_millis(
            fan_control::SAMPLE_PERIOD_MS.into(),
        ))
        .await;

        // Process UI events produced by the button task.
        while let Ok(event) = ui_event_rx.try_receive() {
            match event {
                UiEvent::SwitchToDashboard => {
                    if !matches!(ui_screen, UiScreen::Dashboard) {
                        ui_screen = UiScreen::Dashboard;
                        info!("ui: screen -> dashboard");
                    }
                }
                UiEvent::SwitchToBattDetail => {
                    if !matches!(ui_screen, UiScreen::BattDetail) {
                        ui_screen = UiScreen::BattDetail;
                        info!("ui: screen -> batt_detail");
                    }
                }
            }
        }

        // Blink phase for the battery-detail balancing indicator.
        // UI整体每 1000 ms 重绘一次，所以这里选择 200 ms 作为半周期，
        // 确保每次重绘都能看到交替的 on/off 状态（1 Hz 可感知闪烁）。
        blink_elapsed_ms = blink_elapsed_ms.saturating_add(fan_control::SAMPLE_PERIOD_MS);
        if blink_elapsed_ms >= 200 {
            blink_elapsed_ms -= 200;
            blink_on = !blink_on;
        }

        // UI/采样刷新节奏：约 2 Hz（500 ms 一次）
        adin_elapsed_ms = adin_elapsed_ms.saturating_add(fan_control::SAMPLE_PERIOD_MS);
        if adin_elapsed_ms >= 500 {
            adin_elapsed_ms -= 500;
            // advance SoC display alternation (2-second cadence)
            soc_alt_counter = soc_alt_counter.saturating_add(1);
            if soc_alt_counter >= 2 {
                soc_alt_counter = 0;
                soc_alt_voltage = !soc_alt_voltage;
            }
            log_sc8815_temperature(
                i2c_bus,
                &mut tca,
                &mut sc,
                &mut sc_init_done,
                &mut last_adin_temp_c,
            )
            .await;

            let mut sb_i2c = I2cDevice::new(i2c_bus);
            let smart_batt = read_smart_battery_temperatures(&mut sb_i2c).await;
            sb_temps = smart_batt;

            let pack_temp_c = sb_temps.and_then(|t| t.pack_c);
            let charger_temp_c = sb_temps.and_then(|t| t.charger_c);
            let pack_temp = convert_temp_to_i16(pack_temp_c);
            let charger_temp = convert_temp_to_i16(charger_temp_c);
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
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            let vbat_mv = read_smart_battery_vbat_mv(&mut sb_i2c).await;
            if let Some(v) = vbat_mv {
                sb_last_vbat_mv = Some(v);
            }
            // Pack current (discharge negative, i16 mA from STM32 smart-battery slave).
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            let ibat_ma = read_smart_battery_ibat_ma(&mut sb_i2c).await;

            let now_millis = esp_hal::time::Instant::now()
                .duration_since_epoch()
                .as_millis() as u64;

            let refresh_in_pg = true;
            if refresh_in_pg {
                if let Some(t) = tca.as_mut() {
                    match t.read_in_pg().await {
                        Ok(state) => {
                            if in_pg_read_failed {
                                info!("power: IN_PG read recovered");
                                in_pg_read_failed = false;
                            }
                            if vin_present != state {
                                vin_present = state;
                                // Record the moment of the edge so that charging logic can
                                // enforce a “VIN stable for AC_STABLE_MS” window on resume.
                                vin_state_last_change_ms = now_millis;
                                controller.set_vin_present(state);
                                if state {
                                    charge_skip_adapter_logged = false;
                                }
                            }
                            if last_in_pg_logged != Some(state) {
                                let mut sb_i2c = I2cDevice::new(i2c_bus);
                                match read_smart_battery_state_flags(&mut sb_i2c).await {
                                    Some(flags) => {
                                        let stm_ac = (flags & SB_STATE_FLAG_AC_PRESENT) != 0;
                                        info!(
                                            "power: adapter {} (stm_ac={} flags=0x{:04x})",
                                            if state { "present" } else { "missing" },
                                            stm_ac,
                                            flags
                                        );
                                    }
                                    None => info!(
                                        "power: adapter {} (stm_ac=? read_fail)",
                                        if state { "present" } else { "missing" }
                                    ),
                                }
                                last_in_pg_logged = Some(state);
                            }
                        }
                        Err(_) => {
                            if !in_pg_read_failed {
                                warn!("tca6408a: read IN_PG failed");
                                in_pg_read_failed = true;
                            }
                        }
                    }
                }
            }

            if let Some(temp) = pack_temp_c {
                if sb_temp_pause_active {
                    if temp <= TEMP_RESUME_C {
                        sb_temp_pause_active = false;
                        info!("charge: temperature resume at {=f32}°C", temp);
                    }
                } else if temp >= TEMP_PAUSE_C {
                    sb_temp_pause_active = true;
                    info!("charge: temperature pause at {=f32}°C", temp);
                }
            }

            if now_millis.saturating_sub(sb_cfg_last_verify_ms) >= SB_CFG_VERIFY_INTERVAL_MS {
                sb_cfg_last_verify_ms = now_millis;
                let mut sb_i2c = I2cDevice::new(i2c_bus);
                match read_smart_battery_reg(&mut sb_i2c, SB_REG_CHG_CONFIG).await {
                    Ok(actual) => {
                        if actual != sb_config_value {
                            warn!(
                                "smart-battery: cfg drift detected hw=0x{:02x} expected=0x{:02x}",
                                actual, sb_config_value
                            );
                            let desired = compose_sb_charge_config(
                                SB_AUTO_ENABLED,
                                sb_manual_enable,
                                sb_speed_tier,
                            );
                            let mut sb_i2c = I2cDevice::new(i2c_bus);
                            match write_smart_battery_reg_retry(
                                &mut sb_i2c,
                                SB_REG_CHG_CONFIG,
                                desired,
                            )
                            .await
                            {
                                Ok(()) => {
                                    sb_config_value = desired;
                                    info!("smart-battery: cfg re-applied after drift");
                                }
                                Err(()) => {
                                    warn!("smart-battery: failed to reapply charge config");
                                }
                            }
                        }
                    }
                    Err(()) => warn!("smart-battery: cfg read failed"),
                }
            }

            // Periodic state snapshot for logging / UI (every 10s)
            if now_millis.saturating_sub(sb_last_state_poll_ms) >= SB_STATE_POLL_INTERVAL_MS {
                sb_last_state_poll_ms = now_millis;
                let mut status: Option<u8> = None;
                let mut pause: Option<u8> = None;
                let mut flags: Option<u16> = None;
                let mut cell_mv: [Option<u16>; 5] = [None; 5];
                let mut cells_present: Option<u8> = None;
                let mut sb_i2c = I2cDevice::new(i2c_bus);
                if let Ok(s) = read_smart_battery_reg_retry(&mut sb_i2c, 0x30, 2, 2).await {
                    status = Some(s);
                } else {
                    warn!("sb:state read CHG_STATUS failed");
                }
                if let Ok(p) =
                    read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_CHG_PAUSE_CAUSE, 2, 2).await
                {
                    pause = Some(p);
                } else {
                    warn!("sb:state read CHG_PAUSE_CAUSE failed");
                }
                if let Ok(f_lo) =
                    read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS, 2, 2).await
                {
                    if let Ok(f_hi) =
                        read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS + 1, 2, 2)
                            .await
                    {
                        flags = Some(((f_hi as u16) << 8) | f_lo as u16);
                    }
                } else {
                    warn!("sb:state read STATE_FLAGS failed");
                }
                // Cell voltages (best-effort, non-atomic)
                if let Ok(c) = read_smart_battery_reg_retry(&mut sb_i2c, 0x1F, 2, 2).await {
                    cells_present = Some(c);
                    let count = (c as usize).min(5);
                    for i in 0..count {
                        let base = 0x50u8.wrapping_add((i as u8) * 2);
                        match (
                            read_smart_battery_reg_retry(&mut sb_i2c, base, 2, 2).await,
                            read_smart_battery_reg_retry(&mut sb_i2c, base.wrapping_add(1), 2, 2)
                                .await,
                        ) {
                            (Ok(lo), Ok(hi)) => {
                                cell_mv[i] = Some(((hi as u16) << 8) | lo as u16);
                            }
                            _ => {
                                warn!("sb:state read cell{} failed", i + 1);
                            }
                        }
                    }
                } else {
                    warn!("sb:state read CELLS_PRESENT failed");
                }

                // Cache latest state flags and per-cell voltages for the UI battery detail page.
                last_state_flags = flags;
                last_cells_mv = cell_mv;

                // Periodic smart-battery state snapshot; keep at debug level to reduce noise.
                debug!(
                    "sb:state status=0x{:02x} pause=0x{:02x} flags=0x{:04x}",
                    status.unwrap_or(0xFF),
                    pause.unwrap_or(0xFF),
                    flags.unwrap_or(0xFFFF)
                );
                if let Some(c) = cells_present {
                    debug!(
                        "sb:cells n={} mv={:?}",
                        c,
                        [
                            cell_mv[0].unwrap_or(0),
                            cell_mv[1].unwrap_or(0),
                            cell_mv[2].unwrap_or(0),
                            cell_mv[3].unwrap_or(0),
                            cell_mv[4].unwrap_or(0)
                        ]
                    );
                }
            }

            if sb_temp_pause_active {
                if sb_manual_enable {
                    let desired_config =
                        compose_sb_charge_config(SB_AUTO_ENABLED, false, sb_speed_tier);
                    if sb_config_value != desired_config {
                        let mut sb_i2c = I2cDevice::new(i2c_bus);
                        if write_smart_battery_reg_retry(
                            &mut sb_i2c,
                            SB_REG_CHG_CONFIG,
                            desired_config,
                        )
                        .await
                        .is_ok()
                        {
                            sb_config_value = desired_config;
                            sb_manual_enable = false;
                            info!("charge: disabled due to high temperature");
                        } else {
                            warn!("charge: failed to disable during temperature pause");
                        }
                    }
                }
            } else if let Some(vbat) = vbat_mv.or(sb_last_vbat_mv) {
                // Derive adapter稳定性窗口，并追踪当前决策输入便于现场排查。
                let vin_ok_for_charge = vin_present
                    && now_millis.saturating_sub(vin_state_last_change_ms) >= AC_STABLE_MS;
                info!(
                    "charge: decision vin_present={} vin_ok_for_charge={} vbat={}mV manual={} temp_pause={}",
                    vin_present, vin_ok_for_charge, vbat, sb_manual_enable, sb_temp_pause_active
                );
                if !vin_present {
                    if sb_manual_enable {
                        let desired_config =
                            compose_sb_charge_config(SB_AUTO_ENABLED, false, sb_speed_tier);
                        if sb_config_value != desired_config && {
                            let mut sb_i2c = I2cDevice::new(i2c_bus);
                            write_smart_battery_reg_retry(
                                &mut sb_i2c,
                                SB_REG_CHG_CONFIG,
                                desired_config,
                            )
                            .await
                            .is_ok()
                        } {
                            sb_config_value = desired_config;
                            sb_manual_enable = false;
                            info!("charge: disabled because adapter is missing");
                        }
                    } else if vbat <= CHARGE_START_VBAT_MV && !charge_skip_adapter_logged {
                        info!("charge: skip enable (adapter missing, vbat={=u32}mV)", vbat);
                        charge_skip_adapter_logged = true;
                    }
                } else if !vin_ok_for_charge {
                    // 适配器刚刚恢复或存在抖动：在稳定窗口内禁止重新开启充电，仅输出一次性日志。
                    if vbat <= CHARGE_START_VBAT_MV && !charge_skip_adapter_logged {
                        info!(
                            "charge: skip enable (adapter unstable, vbat={=u32}mV)",
                            vbat
                        );
                        charge_skip_adapter_logged = true;
                    }
                } else {
                    charge_skip_adapter_logged = false;
                    let mut target_manual = sb_manual_enable;
                    if !sb_manual_enable && vbat <= CHARGE_START_VBAT_MV {
                        target_manual = true;
                    }
                    if sb_manual_enable && vbat >= CHARGE_STOP_VBAT_MV {
                        target_manual = false;
                    }

                    if target_manual != sb_manual_enable {
                        let desired_config =
                            compose_sb_charge_config(SB_AUTO_ENABLED, target_manual, sb_speed_tier);
                        let mut sb_i2c = I2cDevice::new(i2c_bus);
                        match write_smart_battery_reg_retry(
                            &mut sb_i2c,
                            SB_REG_CHG_CONFIG,
                            desired_config,
                        )
                        .await
                        {
                            Ok(()) => {
                                sb_config_value = desired_config;
                                sb_manual_enable = target_manual;
                                if target_manual {
                                    info!(
                                        "charge: enabled (vbat={=u32}mV, threshold={=u32}mV)",
                                        vbat, CHARGE_START_VBAT_MV
                                    );
                                } else {
                                    info!(
                                        "charge: disabled at {=u32}mV (stop threshold {=u32}mV)",
                                        vbat, CHARGE_STOP_VBAT_MV
                                    );
                                }
                            }
                            Err(()) => {
                                warn!("charge: failed to update charge config register");
                            }
                        }
                    }
                }
            }
            // Derive UI mode and pack current magnitude from IBAT.
            let (ui_mode, pack_i_ma_abs) = match ibat_ma {
                Some(i) => {
                    let abs_ma: u32 = if i < 0 { (-i) as u32 } else { i as u32 };
                    let mode = if abs_ma < 50 {
                        ui::Mode::Standby
                    } else if i > 0 {
                        ui::Mode::Charge
                    } else {
                        ui::Mode::Discharge
                    };
                    (mode, Some(abs_ma))
                }
                None => (ui::Mode::Standby, None),
            };

            // Best-effort balancing cell index: when the smart-battery reports
            // "balancing active" in STATE_FLAGS, highlight the highest-voltage present cell.
            let balancing_index = if let Some(flags) = last_state_flags {
                if (flags & SB_STATE_FLAG_BALANCING) != 0 {
                    let mut best_idx: Option<usize> = None;
                    let mut best_mv: u16 = 0;
                    for (idx, cell_opt) in last_cells_mv.iter().enumerate() {
                        if let Some(mv) = *cell_opt {
                            if best_idx.is_none() || mv > best_mv {
                                best_idx = Some(idx);
                                best_mv = mv;
                            }
                        }
                    }
                    best_idx.map(|i| (i + 1) as u8)
                } else {
                    None
                }
            } else {
                None
            };

            let soc_pct = vbat_mv.map(|v| estimate_soc_from_vbat(v)).unwrap_or(0);
            let uptime_secs = now_millis
                .saturating_sub(boot_millis)
                .saturating_div(1000)
                .min(u64::from(u32::MAX)) as u32;

            // Build a minimal dashboard model (real temps + SoC; other fields placeholder)
            let model = ui::DashboardData {
                mode: ui_mode,
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

            // Battery detail view model (reuses most of the same raw inputs).
            // Prefer VBAT reading from STM32; if unavailable, fall back to summing per-cell voltages.
            let pack_v_detail = vbat_mv.or_else(|| {
                let mut total: u32 = 0;
                for cell_opt in last_cells_mv.iter() {
                    if let Some(mv) = *cell_opt {
                        total = total.saturating_add(mv as u32);
                    } else {
                        return None;
                    }
                }
                Some(total)
            });

            let batt_detail = ui::BattDetailData {
                mode: ui_mode,
                pack_v_mv: pack_v_detail,
                pack_i_ma: pack_i_ma_abs,
                cells_mv: last_cells_mv,
                balancing_index,
                temps_c: [pack_temp, charger_temp, adin_temp, None],
                blink_on,
            };

            match ui_screen {
                UiScreen::Dashboard => {
                    let _ =
                        ui::render_dashboard_once_async(&mut spi, &mut _cs, &mut _dc, &model).await;
                }
                UiScreen::BattDetail => {
                    let _ = ui::render_batt_detail_once_async(
                        &mut spi,
                        &mut _cs,
                        &mut _dc,
                        &batt_detail,
                    )
                    .await;
                }
            }
        }
    }
}

async fn log_sc8815_temperature(
    i2c_bus: &'static I2cBusMutex,
    tca: &mut Option<Tca6408a<SharedI2cDevice<'static>>>,
    sc: &mut Option<sc8815::SC8815<SharedI2cDevice<'static>>>,
    sc_init_done: &mut bool,
    last_adin_temp_c: &mut Option<f32>,
) {
    let mut adin_mv_sample: Option<u16> = None;
    let mut ce_enabled = false;

    if let Some(t) = tca.as_mut() {
        if let Err(_) = t.set_sc_ce(true).await {
            warn!("sc8815: failed to pull CE low via TCA6408A");
        } else {
            ce_enabled = true;
        }
    }

    Timer::after(Duration::from_millis(5)).await;

    if ce_enabled {
        // Ensure instance exists
        if sc.is_none() {
            let dev = I2cDevice::new(i2c_bus);
            let mut drv = sc8815::SC8815::new(dev, SC8815_ADDR);
            if !*sc_init_done {
                match drv.init().await {
                    Ok(()) => *sc_init_done = true,
                    Err(_) => warn!("sc8815: init failed"),
                }
            }
            *sc = Some(drv);
        }

        if let Some(d) = sc.as_mut() {
            if let Err(_) = d.set_adc_conversion(true).await {
                warn!("sc8815: ADC start failed");
            } else {
                Timer::after(Duration::from_millis(10)).await;
                if let Ok(meas) = d.get_adc_measurements().await {
                    adin_mv_sample = Some(meas.adin_mv);
                }
                let _ = d.set_adc_conversion(false).await;
            }
        }
    }

    if let Some(t) = tca.as_mut() {
        if ce_enabled {
            if let Err(_) = t.set_sc_ce(false).await {
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

async fn read_smart_battery_temperatures<I2C>(
    i2c: &mut I2C,
) -> Option<fan_control::SmartBatteryTemps>
where
    I2C: AsyncI2c,
{
    // STM32 smart-battery slave currently exposes raw registers without CRC interleaving.
    // Use pointer write then read 4 bytes: [TPACK_L, TPACK_H, TCHG_L, TCHG_H] (i16 LE, 0.01°C; i16::MIN = invalid).
    let mut data = [0u8; 4];
    match i2c.write_read(STM32_ADDR, &[SB_TEMP_BASE], &mut data).await {
        Ok(()) => {
            let pack = i16::from_le_bytes([data[0], data[1]]);
            let charger = i16::from_le_bytes([data[2], data[3]]);
            let pack_c = temp_from_centi(pack);
            let charger_c = temp_from_centi(charger);
            let temps = fan_control::SmartBatteryTemps::new(pack_c, charger_c);
            // Keep detailed temperature reporting at debug level to avoid cluttering button logs.
            debug!(
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

async fn read_smart_battery_vbat_mv<I2C>(i2c: &mut I2C) -> Option<u32>
where
    I2C: AsyncI2c,
{
    let mut vbuf = [0u8; 2];
    match i2c.write_read(STM32_ADDR, &[0x10], &mut vbuf).await {
        Ok(()) => {
            let v = u16::from_le_bytes(vbuf) as u32;
            Some(v)
        }
        Err(_) => {
            warn!("stm32: vbat read failed");
            None
        }
    }
}

async fn read_smart_battery_ibat_ma<I2C>(i2c: &mut I2C) -> Option<i32>
where
    I2C: AsyncI2c,
{
    // IBAT is exposed as i16 in milliamps; discharge is negative.
    let mut buf = [0u8; 2];
    if i2c.write_read(STM32_ADDR, &[0x12], &mut buf).await.is_ok() {
        let i = i16::from_le_bytes(buf) as i32;
        Some(i)
    } else {
        None
    }
}

async fn read_smart_battery_state_flags<I2C>(i2c: &mut I2C) -> Option<u16>
where
    I2C: AsyncI2c,
{
    let mut buf = [0u8; 2];
    i2c.write_read(STM32_ADDR, &[SB_REG_STATE_FLAGS], &mut buf)
        .await
        .ok()
        .map(|_| u16::from_le_bytes(buf))
}

async fn read_smart_battery_reg<I2C>(i2c: &mut I2C, reg: u8) -> Result<u8, ()>
where
    I2C: AsyncI2c,
{
    let mut buf = [0u8; 1];
    i2c.write_read(STM32_ADDR, &[reg], &mut buf)
        .await
        .map_err(|_| ())?;
    Ok(buf[0])
}

async fn write_smart_battery_reg<I2C>(i2c: &mut I2C, reg: u8, value: u8) -> Result<(), ()>
where
    I2C: AsyncI2c,
{
    i2c.write(STM32_ADDR, &[reg, value]).await.map_err(|_| ())
}

async fn write_smart_battery_reg_retry<I2C>(i2c: &mut I2C, reg: u8, value: u8) -> Result<(), ()>
where
    I2C: AsyncI2c,
{
    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match write_smart_battery_reg(i2c, reg, value).await {
            Ok(()) => return Ok(()),
            Err(_) if attempt < SB_WRITE_RETRY_ATTEMPTS => {
                Timer::after(Duration::from_millis(SB_WRITE_RETRY_DELAY_MS.into())).await;
            }
            Err(_) => return Err(()),
        }
    }
}

async fn read_smart_battery_reg_retry<I2C>(
    i2c: &mut I2C,
    reg: u8,
    attempts: u8,
    delay_ms: u32,
) -> Result<u8, ()>
where
    I2C: AsyncI2c,
{
    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match read_smart_battery_reg(i2c, reg).await {
            Ok(v) => return Ok(v),
            Err(_) if attempt < attempts => {
                Timer::after(Duration::from_millis(delay_ms.into())).await;
            }
            Err(_) => return Err(()),
        }
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

async fn stm_one_shot_validate<I2C>(i2c: &mut I2C) -> Result<(), ()>
where
    I2C: AsyncI2c,
{
    // Write window byte at 0x08
    let set_a = [SB_WINDOW_START, TEST_A];
    i2c.write(STM32_ADDR, &set_a).await.map_err(|_| ())?;

    // Read 16 bytes from 0x00, confirm signature and window value
    let mut buf = [0u8; 16];
    i2c.write_read(STM32_ADDR, &[0x00], &mut buf)
        .await
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
    i2c.write(STM32_ADDR, &set_tail).await.map_err(|_| ())?;
    let mut tail = [0u8; 4];
    i2c.write_read(STM32_ADDR, &[SB_WINDOW_END - 1], &mut tail)
        .await
        .map_err(|_| ())?;
    info!("stm32: tail={=[u8]:02x}", &tail[..]);
    if !(tail[0] == TEST_A && tail[1] == TEST_B) {
        return Err(());
    }

    Ok(())
}
