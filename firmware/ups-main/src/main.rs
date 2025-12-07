#![no_std]
#![no_main]

mod adin_temp;
mod batt_est;
mod button_input;
mod display;
mod fan_control;
mod io_expander;
mod power;
mod thermal;
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
    Async,
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    dma_buffers,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed, channel, channel::ChannelIFace, timer, timer::TimerIFace,
    },
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi, SpiDmaBus},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_println as _; // install UART logger + defmt bridge
use io_expander::Tca6408a;
use sc8815::{self, registers::constants::DEFAULT_ADDRESS as SC8815_ADDR};
use static_cell::StaticCell;
// STM32 smart-battery I2C slave (validation-only, single-shot)
pub const STM32_ADDR: u8 = 0x35;
pub const SB_SIG: [u8; 2] = [b'S', b'B'];
pub const SB_WINDOW_START: u8 = 0x08;
pub const SB_WINDOW_END: u8 = 0x0F;
// Compact temperature telemetry window (0x40..0x47, all int8 in °C) sourced
// from the STM32 smart-battery thermal aggregation and used as the sole
// temperature source on the ESP32 side:
//   [0]=pack, [1]=charger, [2..5]=NTC0..3, [6]=BQ_INT, [7]=MCU.
pub const SB_TEMP_WINDOW_BASE: u8 = 0x40;
pub const SB_TEMP_WINDOW_LEN: usize = 8;
pub const TEST_A: u8 = 0x5A;
pub const TEST_B: u8 = 0xA5;
pub const SB_REG_CHG_CONFIG: u8 = 0x31;
pub const SB_REG_CHG_PAUSE_CAUSE: u8 = 0x32;
pub const SB_REG_STATE_FLAGS: u8 = 0x20;
/// Smart-battery TEMP_STATUS register (single-byte thermal fault flags).
/// See `TempFaultFlags` / `decode_temp_status` for the decoded view.
pub const SB_REG_TEMP_STATUS: u8 = 0x23;
pub const SB_STATE_FLAG_AC_PRESENT: u16 = 0x0001;
// Mirror of STM32 smart-battery state_bits::BALANCING; used for UI overlay.
pub const SB_STATE_FLAG_BALANCING: u16 = 1 << 5;
// Mirrors of smart-battery state_bits::FAULT_BQ / FAULT_SC; used for UPS discharge gating.
pub const SB_STATE_FLAG_FAULT_BQ: u16 = 1 << 6;
pub const SB_STATE_FLAG_FAULT_SC: u16 = 1 << 7;

/// Decoded view of the STM32 smart-battery TEMP_STATUS register (0x23).
#[derive(Clone, Copy, Debug)]
pub struct TempFaultFlags {
    pub temp_low: bool,
    pub temp_high_chg: bool,
    pub temp_high_dsg: bool,
}

/// Decode TEMP_STATUS bits into a strongly-typed flag set.
pub fn decode_temp_status(raw: u8) -> TempFaultFlags {
    TempFaultFlags {
        temp_low: (raw & 0x01) != 0,
        temp_high_chg: (raw & 0x02) != 0,
        temp_high_dsg: (raw & 0x04) != 0,
    }
}

pub const SB_CHG_STATUS_BALANCING: u8 = 1 << 5;
pub const SB_CFG_BIT_AUTO: u8 = 1 << 0;
pub const SB_CFG_BIT_MANUAL: u8 = 1 << 1;
pub const SB_CFG_SPEED_SHIFT: u8 = 2;

// Battery pack configuration (per project spec; do not probe at runtime)
pub const PACK_CELLS_S: u8 = 5; // 5S Li-ion (BQ76920 max 5S)
pub const SOC_EMPTY_VBAT_MV: u32 = 12_500; // Cutoff threshold (pack)
pub const SOC_FULL_VBAT_MV: u32 = 18_500; // Full threshold (pack)
pub const CHARGE_START_VBAT_MV: u32 = 17_000;
pub const CHARGE_STOP_VBAT_MV: u32 = SOC_FULL_VBAT_MV;
/// UPS discharge low-voltage stop threshold (pack), must stay >= PACK_OUTPUT_CUTOFF_THRESHOLD_MV.
/// match discharge_policy.md §4.3; 5S * 2.7V ≈ 13.5V, keeps margin above PACK_OUTPUT_CUTOFF_THRESHOLD_MV.
pub const DISCH_STOP_VBAT_MV: u32 = 13_500;
/// UPS discharge resume threshold with hysteresis (pack).
/// match discharge_policy.md §4.3; 5S * 3.2V ≈ 16.0V.
pub const DISCH_RESUME_VBAT_MV: u32 = 16_000;
/// AC 适配器恢复后，IN_PG 连续为 High 至少 10 s 才允许重新开始充电。
pub const AC_STABLE_MS: u64 = 10_000;
pub const TEMP_PAUSE_C: f32 = 40.0;
pub const TEMP_RESUME_C: f32 = 35.0;
/// UPS discharge thermal stop threshold (SC8815 ADIN, °C).
pub const UPS_DISCH_STOP_C: f32 = 70.0;
/// UPS discharge thermal resume threshold (°C), must be < UPS_DISCH_STOP_C.
pub const UPS_DISCH_RESUME_C: f32 = 50.0;
/// UPS VBUS minimum/maximum targets allowed by project spec (see SC8815_External_Resistor_Configuration.md).
pub const UPS_VBUS_MIN_MV: u16 = 9_000;
pub const UPS_VBUS_MAX_MV: u16 = 20_600;
/// SC8815 VBUS/VBAT sense resistor values on the UPS power board (see
/// docs/ups-power-board/netlist_ups-power-board.enet, R47/R26 = 5mΩ).
pub const UPS_SC_RS1_MOHM: u16 = 5;
pub const UPS_SC_RS2_MOHM: u16 = 5;
/// SC8815 OTG current limits (mA) for UPS OUT path.
/// With RS1=RS2=5mΩ and IBUS_RATIO=3x, the SC8815 datasheet and driver
/// formula cap IBUS at ≈6A (IBUS_LIM_SET <= 255). We therefore clamp the
/// software limit to 6000mA on both sides.
pub const UPS_SC_IBUS_LIMIT_MA: u16 = 6_000;
pub const UPS_SC_IBAT_LIMIT_MA: u16 = 6_000;
/// Default 12V mode targets (VIN present vs missing).
pub const UPS_VBUS_AC_ONLINE_MV: u16 = 11_500;
pub const UPS_VBUS_AC_OFFLINE_MV: u16 = 12_000;
pub const SB_WRITE_RETRY_ATTEMPTS: u8 = 3;
pub const SB_WRITE_RETRY_DELAY_MS: u32 = 5;
pub const SB_CFG_VERIFY_INTERVAL_MS: u64 = 1_000;
pub const SB_STATE_POLL_INTERVAL_MS: u64 = 10_000;

pub type I2cBusMutex = Mutex<NoopRawMutex, I2c<'static, Async>>;
pub type SharedI2cDevice<'a> = I2cDevice<'a, NoopRawMutex, I2c<'static, Async>>;
static I2C0_BUS: StaticCell<I2cBusMutex> = StaticCell::new();

static FAN_TIMER: StaticCell<timer::Timer<'static, LowSpeed>> = StaticCell::new();

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

pub(crate) fn compose_sb_charge_config(auto: bool, manual: bool, speed_tier: u8) -> u8 {
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

/// Asynchronous UI task responsible for:
///   * Consuming navigation events from the button task.
///   * Periodically sampling power/thermal state snapshots for display.
///   * Rendering full-frame dashboard / battery-detail screens over async SPI.
#[embassy_executor::task]
async fn ui_task(
    mut spi: SpiDmaBus<'static, Async>,
    mut cs: Output<'static>,
    mut dc: Output<'static>,
    ui_event_rx: UiEventReceiver,
    power_state: &'static power::PowerStateMutex,
    thermal_state: &'static thermal::ThermalStateMutex,
    boot_millis: u64,
) {
    // UI navigation state: dashboard vs battery detail.
    let mut ui_screen = UiScreen::Dashboard;

    // UI: alternate SoC% and VBAT every ~2 seconds.
    let mut adin_elapsed_ms: u32 = 0;
    let mut soc_alt_counter: u8 = 0; // seconds within current phase
    let mut soc_alt_voltage: bool = false; // false => show %, true => show voltage

    // UI: rotate through available temperature sources every ~2 seconds.
    let mut temp_alt_counter: u8 = 0;
    let mut temp_cycle_index: usize = 0;

    // UI: batt-detail cells frame (voltages vs temperatures) alternation (~2 seconds).
    let mut cells_frame = ui::CellsFrame::Voltage;
    let mut cells_alt_counter: u8 = 0;

    loop {
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

            // advance batt-detail cells frame alternation (~2-second cadence)
            cells_alt_counter = cells_alt_counter.saturating_add(1);
            if cells_alt_counter >= 4 {
                cells_alt_counter = 0;
                cells_frame = match cells_frame {
                    ui::CellsFrame::Voltage => ui::CellsFrame::Temp,
                    ui::CellsFrame::Temp => ui::CellsFrame::Voltage,
                };
            }

            // Snapshot power and thermal readings from background tasks instead of
            // performing I2C transactions or TSENS reads directly here.
            let power_snapshot = {
                let state = power_state.lock().await;
                *state
            };
            let thermal_snapshot = {
                let state = thermal_state.lock().await;
                *state
            };
            let last_state_flags = power_snapshot.state_flags;
            let last_cells_mv = power_snapshot.cells_mv;

            let pack_temp = convert_temp_to_i16(thermal_snapshot.sb_pack_temp_c);
            let charger_temp = convert_temp_to_i16(thermal_snapshot.sb_charger_temp_c);
            let ups_temp = convert_temp_to_i16(thermal_snapshot.ups_temp_c);
            let cell_temps = estimate_cell_temps_i16(thermal_snapshot.sb_ntc_temps_c);

            let temp_sources = [
                (ui::TempSlot::Battery, pack_temp),
                (ui::TempSlot::Charger, charger_temp),
                (ui::TempSlot::Ups, ups_temp),
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

            // Use latest VBAT / IBAT / OUT measurements from power_task.
            let vbat_mv = power_snapshot.vbat_mv;
            let ibat_ma = power_snapshot.ibat_ma;
            let out_enabled = power_snapshot.out_enabled;

            let now_millis = esp_hal::time::Instant::now()
                .duration_since_epoch()
                .as_millis() as u64;

            // Derive UI mode and pack current magnitude from IBAT.
            let (mut ui_mode, pack_i_ma_abs) = match ibat_ma {
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

            // Ensure Mode::Discharge is only shown when OUT is actually enabled
            // (match discharge_policy.md §7 UI wiring note).
            if matches!(ui_mode, ui::Mode::Discharge) && !out_enabled {
                ui_mode = ui::Mode::Standby;
            }

            // Best-effort balancing cell index: when the smart-battery reports
            // \"balancing active\" in STATE_FLAGS, highlight the highest-voltage present cell.
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

            // OUT trio values for dashboard. When OUT is disabled or missing,
            // keep them at 0 so the Discharge view renders a neutral value.
            let out_v_mv = power_snapshot.out_v_mv.unwrap_or(0);
            let out_a_ma = power_snapshot
                .out_a_ma
                .map(|i| if i < 0 { (-i) as u32 } else { i as u32 })
                .unwrap_or(0);
            let out_w_mw = power_snapshot.out_w_mw.unwrap_or(0);

            // Build a minimal dashboard model (real temps + SoC; other fields placeholder).
            let model = ui::DashboardData {
                mode: ui_mode,
                soc_pct,
                vbat_mv,
                soc_display: if soc_alt_voltage {
                    ui::SocDisplay::Voltage
                } else {
                    ui::SocDisplay::Percent
                },
                in_v_mv: 0,
                in_a_ma: 0,
                in_w_mw: 0,
                chg_w_mw: 0,
                out_v_mv,
                out_a_ma,
                out_w_mw,
                bat_temp_c: pack_temp,
                charger_temp_c: charger_temp,
                ups_temp_c: ups_temp,
                fan_pct: thermal_snapshot.fan.duty_pct,
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
                cells_temp_c: cell_temps,
                balancing_index,
                temp_fault: power_snapshot.sb_temp_status.map(decode_temp_status),
            };

            match ui_screen {
                UiScreen::Dashboard => {
                    let _ =
                        ui::render_dashboard_once_async(&mut spi, &mut cs, &mut dc, &model).await;
                }
                UiScreen::BattDetail => {
                    let _ = ui::render_batt_detail_once_async(
                        &mut spi,
                        &mut cs,
                        &mut dc,
                        &batt_detail,
                        cells_frame,
                    )
                    .await;
                }
            }
        }

        // Maintain the legacy 20 ms UI loop cadence so other tasks (e.g.
        // button_task, power_task, thermal_task) can make progress between
        // iterations.
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

    // Initialise shared power/thermal state and spawn background management tasks.
    let power_state = power::init_power_state();
    let thermal_state = thermal::init_thermal_state();
    let _ = spawner.spawn(power::power_task(i2c_bus, power_state, thermal_state));

    // SPI for LCD (write-only): MOSI=11, SCLK=12, optional CS/DC/RST/BL as GPIO
    let mut _dc = Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default());
    let mut _cs = Output::new(peripherals.GPIO13, Level::High, OutputConfig::default());
    let mut _rst = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    // Backlight via LEDC will be configured below; keep BL pin as PWM (GPIO15)
    let _ = _rst.set_low();
    delay.delay_ms(10u32);
    let _ = _rst.set_high();

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

    let t_fan = FAN_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
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
            timer: t_fan,
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

    // Indicate power-management bring-up on the boot screen; the actual
    // I2C operations are performed in the background `power_task`.
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 40, "TCA6408A");
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 60, "SC8815 ADC Read");

    // === Temperature sensor init (TSENS delta calibration only) ===
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

    // Spawn dedicated thermal-management task owning the fan controller.
    let controller = fan_control::FanController::new(fan_pwm, fan_en, delta_opt, false);
    let _ = spawner.spawn(thermal::thermal_task(
        controller,
        power_state,
        thermal_state,
    ));
    let _ = ui::boot_update(&mut spi, &mut _cs, &mut _dc, 100, "Ready");

    // Hand LCD ownership to the dedicated UI task; it will drive periodic
    // full-frame updates using async SPI and shared state snapshots.
    let _ = spawner.spawn(ui_task(
        spi,
        _cs,
        _dc,
        ui_event_rx,
        power_state,
        thermal_state,
        boot_millis,
    ));

    // Park the main task; all work is now handled by background Embassy tasks.
    loop {
        Timer::after(Duration::from_millis(1_000)).await;
    }
}

fn i2c_error_kind_str<E>(e: &E) -> &'static str
where
    E: embedded_hal::i2c::Error,
{
    use embedded_hal::i2c::ErrorKind;
    match e.kind() {
        ErrorKind::Bus => "bus",
        ErrorKind::ArbitrationLoss => "arbitration-loss",
        ErrorKind::NoAcknowledge(_) => "nack",
        ErrorKind::Overrun => "overrun",
        _ => "other",
    }
}

pub(crate) async fn read_smart_battery_temperatures<I2C>(
    i2c: &mut I2C,
) -> Option<fan_control::SmartBatteryTemps>
where
    I2C: AsyncI2c,
    <I2C as embedded_hal::i2c::ErrorType>::Error: embedded_hal::i2c::Error,
{
    let now_ms = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;
    // Compact temperature window (0x40..0x47, int8 °C). Layout:
    //   [0]=pack, [1]=charger, [2..5]=NTC0..3, [6]=BQ_INT, [7]=MCU.
    match read_smart_battery_temp_window(i2c).await {
        Ok([pack_i8, chg_i8, n0, n1, n2, n3, bq_int_i8, mcu_i8]) => {
            let to_opt =
                |v: i8| -> Option<f32> { if v == i8::MIN { None } else { Some(v as f32) } };

            let pack_c = to_opt(pack_i8);
            let charger_c = to_opt(chg_i8);
            let mut ntc_c: [Option<f32>; 4] = [None; 4];
            ntc_c[0] = to_opt(n0);
            ntc_c[1] = to_opt(n1);
            ntc_c[2] = to_opt(n2);
            ntc_c[3] = to_opt(n3);

            // Keep BQ/MCU temperatures available for potential future diagnostics.
            debug!(
                "stm32: temp-window raw pack={}C chg={}C ntc=[{}, {}, {}, {}] bq_int={}C mcu={}C",
                pack_i8, chg_i8, n0, n1, n2, n3, bq_int_i8, mcu_i8
            );

            let temps = fan_control::SmartBatteryTemps::new(pack_c, charger_c, ntc_c);
            // Keep detailed temperature reporting at debug level to avoid cluttering logs.
            debug!(
                "stm32: temps pack={=f32}°C chg={=f32}°C highest={=f32}°C",
                pack_c.unwrap_or(f32::NAN),
                charger_c.unwrap_or(f32::NAN),
                temps.highest().unwrap_or(f32::NAN)
            );
            Some(temps)
        }
        Err(e) => {
            let kind = i2c_error_kind_str(&e);
            warn!(
                "stm32: temp-window read failed: kind={} t_ms={}",
                kind, now_ms
            );
            None
        }
    }
}

pub(crate) async fn read_smart_battery_vbat_mv<I2C>(i2c: &mut I2C) -> Option<u32>
where
    I2C: AsyncI2c,
    <I2C as embedded_hal::i2c::ErrorType>::Error: embedded_hal::i2c::Error,
{
    let now_ms = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;
    let mut vbuf = [0u8; 2];
    match i2c.write_read(STM32_ADDR, &[0x10], &mut vbuf).await {
        Ok(()) => {
            let v = u16::from_le_bytes(vbuf) as u32;
            Some(v)
        }
        Err(e) => {
            let kind = i2c_error_kind_str(&e);
            warn!("stm32: vbat read failed: kind={} t_ms={}", kind, now_ms);
            None
        }
    }
}

pub(crate) async fn read_smart_battery_ibat_ma<I2C>(i2c: &mut I2C) -> Option<i32>
where
    I2C: AsyncI2c,
    <I2C as embedded_hal::i2c::ErrorType>::Error: embedded_hal::i2c::Error,
{
    let now_ms = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;
    // IBAT is exposed as i16 in milliamps; discharge is negative.
    let mut buf = [0u8; 2];
    match i2c.write_read(STM32_ADDR, &[0x12], &mut buf).await {
        Ok(()) => {
            let i = i16::from_le_bytes(buf) as i32;
            Some(i)
        }
        Err(e) => {
            let kind = i2c_error_kind_str(&e);
            warn!("stm32: ibat read failed: kind={} t_ms={}", kind, now_ms);
            None
        }
    }
}

pub(crate) async fn read_smart_battery_state_flags<I2C>(i2c: &mut I2C) -> Option<u16>
where
    I2C: AsyncI2c,
    <I2C as embedded_hal::i2c::ErrorType>::Error: embedded_hal::i2c::Error,
{
    let now_ms = esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis() as u64;
    let mut buf = [0u8; 2];
    match i2c
        .write_read(STM32_ADDR, &[SB_REG_STATE_FLAGS], &mut buf)
        .await
    {
        Ok(()) => Some(u16::from_le_bytes(buf)),
        Err(e) => {
            let kind = i2c_error_kind_str(&e);
            warn!(
                "stm32: state-flags read failed: kind={} t_ms={}",
                kind, now_ms
            );
            None
        }
    }
}

/// Diagnostic helper: read the STM32 smart-battery compact temperature window
/// (0x40..0x47, all int8 in °C) and return the decoded values as signed
/// degrees Celsius. Layout matches the STM32-side `therm:` log:
///   [0]=pack, [1]=charger, [2..5]=NTC0..3, [6]=BQ_INT, [7]=MCU.
pub(crate) async fn read_smart_battery_temp_window<I2C>(
    i2c: &mut I2C,
) -> Result<[i8; SB_TEMP_WINDOW_LEN], <I2C as embedded_hal::i2c::ErrorType>::Error>
where
    I2C: AsyncI2c,
    <I2C as embedded_hal::i2c::ErrorType>::Error: embedded_hal::i2c::Error,
{
    let mut buf = [0u8; SB_TEMP_WINDOW_LEN];
    i2c.write_read(STM32_ADDR, &[SB_TEMP_WINDOW_BASE], &mut buf)
        .await?;

    let mut out = [0i8; SB_TEMP_WINDOW_LEN];
    for (i, b) in buf.iter().enumerate() {
        out[i] = *b as i8;
    }
    Ok(out)
}

pub(crate) async fn read_smart_battery_reg<I2C>(i2c: &mut I2C, reg: u8) -> Result<u8, ()>
where
    I2C: AsyncI2c,
{
    let mut buf = [0u8; 1];
    i2c.write_read(STM32_ADDR, &[reg], &mut buf)
        .await
        .map_err(|_| ())?;
    Ok(buf[0])
}

pub(crate) async fn write_smart_battery_reg<I2C>(
    i2c: &mut I2C,
    reg: u8,
    value: u8,
) -> Result<(), ()>
where
    I2C: AsyncI2c,
{
    i2c.write(STM32_ADDR, &[reg, value]).await.map_err(|_| ())
}

pub(crate) async fn write_smart_battery_reg_retry<I2C>(
    i2c: &mut I2C,
    reg: u8,
    value: u8,
) -> Result<(), ()>
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

pub(crate) async fn read_smart_battery_reg_retry<I2C>(
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

/// Estimate 5 cell-surface temperatures (degC) from 4 NTC probes using a simple
/// positional model:
/// - NTC0: between cells 1–2
/// - NTC1: between cells 2–3
/// - NTC2: between cells 3–4
/// - NTC3: between cells 4–5
///
/// Mapping (in integer domain, after per-probe conversion):
/// - T1 = NTC0
/// - T2 = avg(NTC0, NTC1)
/// - T3 = avg(NTC1, NTC2)
/// - T4 = avg(NTC2, NTC3)
/// - T5 = NTC3
///
/// avg(a, b):
/// - both Some: rounded average
/// - one Some: that value
/// - both None: None
fn estimate_cell_temps_i16(ntc_c: [Option<f32>; 4]) -> [Option<i16>; 5] {
    fn avg_i16(a: Option<i16>, b: Option<i16>) -> Option<i16> {
        match (a, b) {
            (Some(x), Some(y)) => {
                let sum = x as i32 + y as i32;
                // Round to nearest, keeping behavior symmetric around zero.
                let avg = if sum >= 0 {
                    (sum + 1) / 2
                } else {
                    (sum - 1) / 2
                };
                Some(avg as i16)
            }
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        }
    }

    let ntc_i16: [Option<i16>; 4] = [
        convert_temp_to_i16(ntc_c[0]),
        convert_temp_to_i16(ntc_c[1]),
        convert_temp_to_i16(ntc_c[2]),
        convert_temp_to_i16(ntc_c[3]),
    ];

    let t1 = ntc_i16[0];
    let t2 = avg_i16(ntc_i16[0], ntc_i16[1]);
    let t3 = avg_i16(ntc_i16[1], ntc_i16[2]);
    let t4 = avg_i16(ntc_i16[2], ntc_i16[3]);
    let t5 = ntc_i16[3];

    [t1, t2, t3, t4, t5]
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

pub(crate) async fn stm_one_shot_validate<I2C>(i2c: &mut I2C) -> Result<(), ()>
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
