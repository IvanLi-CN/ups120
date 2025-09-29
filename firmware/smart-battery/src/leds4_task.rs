//! 4-LED status task per SOFTWARE_DESIGN.md (3 s base cycle)
//!
//! LED map: 1→4 = Red, Yellow, Green, Blue.
//! Active-low GPIOs: low = ON, high = OFF.
//!
//! Priorities per LED: dropout 1 Hz > fault 50% > base + pulses.
//! Green additionally supports an async one-shot pulse on I2C1 activity.

use embassy_stm32::gpio::OutputOpenDrain;
use embassy_time::{Duration, Instant, Timer};

use crate::global_state::BatteryGlobalState;
use crate::shared::{Bq76920MeasurementsSubscriber, GlobalStateSubscriber};

// Timing (3 s base cycle; pulses are 120 ms wide with 120 ms gaps)
const CYCLE_MS: u32 = 3000;
const FAULT_ON_MS: u32 = 1500; // 50% blink window
const PULSE_W_MS: u32 = 120;
const PULSE_GAP_MS: u32 = 120;

#[derive(Clone, Copy, Default)]
struct LedIntent {
    base_on: bool,   // true → LED ON (low level)
    pulses: u8,      // 0..=8 pulses at cycle start
    fault_blink: bool,
    dropout_blink: bool, // reserved; to be driven by dropout counters
}

pub struct LedPins {
    pub red: OutputOpenDrain<'static>,
    pub yellow: OutputOpenDrain<'static>,
    pub green: OutputOpenDrain<'static>,
    pub blue: OutputOpenDrain<'static>,
}

fn apply_led(pin: &mut OutputOpenDrain<'static>, on: bool) {
    if on {
        pin.set_low();
    } else {
        pin.set_high();
    }
}

fn compose_with_pulses(base_on: bool, pulses: u8, phase_ms: u32) -> bool {
    if pulses == 0 {
        return base_on;
    }
    let window_ms = (PULSE_W_MS + PULSE_GAP_MS) as u32;
    let slot = (phase_ms / window_ms) as u8;
    if slot < pulses {
        // inside pulse band
        let in_pulse = (phase_ms % window_ms) < PULSE_W_MS;
        return if in_pulse { !base_on } else { base_on };
    }
    base_on
}

fn bq_fets_both_on<const N: usize>(bq_opt: &Option<crate::data_types::Bq76920Measurements<N>>) -> bool {
    if let Some(bq) = bq_opt.as_ref() {
        use bq769x0_async_rs::registers::SysCtrl2Flags;
        let m = bq.core_measurements.mos_status.0;
        return m.contains(SysCtrl2Flags::CHG_ON) && m.contains(SysCtrl2Flags::DSG_ON);
    }
    false
}

/// 4-LED status compositor
#[embassy_executor::task]
pub async fn leds_task(
    mut pins: LedPins,
    mut gs_sub: GlobalStateSubscriber<'static>,
    mut bq_sub: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    // Per-LED 3 s epochs
    let mut epoch_red = Instant::now();
    let mut epoch_yellow = epoch_red;
    let mut epoch_green = epoch_red;
    let mut epoch_blue = epoch_red;

    // Async green pulse state
    let mut green_pulse_until: Option<Instant> = None;

    // Latest sources
    let mut gs: BatteryGlobalState = BatteryGlobalState::default();
    let mut bq: Option<crate::data_types::Bq76920Measurements<5>> = None;

    loop {
        // Non-blocking drains
        if let Some(s) = gs_sub.try_next_message_pure() {
            gs = s;
        }
        if let Some(m) = bq_sub.try_next_message_pure() {
            bq = Some(m);
        }

        // Trigger async green pulse on I2C1 activity
        if crate::activity::I2C1_ACTIVITY_PULSE.load(core::sync::atomic::Ordering::Relaxed) {
            crate::activity::I2C1_ACTIVITY_PULSE.store(false, core::sync::atomic::Ordering::Relaxed);
            green_pulse_until = Some(Instant::now() + Duration::from_millis(PULSE_W_MS as u64));
        }

        let now = Instant::now();
        let p_red = (now - epoch_red).as_millis() as u32 % CYCLE_MS;
        let p_yellow = (now - epoch_yellow).as_millis() as u32 % CYCLE_MS;
        let p_green = (now - epoch_green).as_millis() as u32 % CYCLE_MS;
        let p_blue = (now - epoch_blue).as_millis() as u32 % CYCLE_MS;

        // Red (BQ FET/Protection)
        let mut red = LedIntent::default();
        // Base: both CHG & DSG ON => OFF; else ON
        let both_on = bq_fets_both_on(&bq);
        red.base_on = !both_on;
        // Fault blink mapping (simplified at step 1): battery fault → 50% blink
        if gs.fault_battery {
            red.fault_blink = true;
        }
        // Pulse: 1 = balancing active (from global state overlay)
        if gs.balancing_active { red.pulses = 1; }

        // Yellow (SC8815)
        let mut yellow = LedIntent::default();
        yellow.base_on = gs.charging; // charging → ON, otherwise OFF
        if gs.fault_charger { yellow.fault_blink = true; }
        // 1-pulse when preparing (conditions met but AC absent)
        if gs.preparing { yellow.pulses = yellow.pulses.max(1); }

        // Green (Comm + Sleep)
        let mut green = LedIntent::default();
        green.base_on = !crate::sleep_manager::is_sleeping();
        // async pulse handled after composition

        // Blue (Global)
        let mut blue = LedIntent::default();
        // Must not be pulse-less; if no pulse applies, show OFF
        blue.base_on = false;
        // Suggested codebook mapping
        if gs.fault_battery || gs.fault_charger {
            blue.pulses = blue.pulses.max(6);
        } else if gs.full {
            blue.pulses = blue.pulses.max(5);
        } else if gs.balancing_active {
            blue.pulses = blue.pulses.max(4);
        } else if gs.charging_paused {
            blue.pulses = blue.pulses.max(3);
        } else if gs.charging {
            blue.pulses = blue.pulses.max(2);
        } else {
            blue.pulses = blue.pulses.max(1);
        }

        // Drive pins with priority
        // RED
        let red_on = if red.dropout_blink {
            // 1 Hz blink (reserve; not set yet)
            (p_red / 500) % 2 == 0
        } else if red.fault_blink {
            p_red < FAULT_ON_MS
        } else {
            compose_with_pulses(red.base_on, red.pulses, p_red)
        };
        apply_led(&mut pins.red, red_on);

        // YELLOW
        let yellow_on = if yellow.dropout_blink {
            (p_yellow / 500) % 2 == 0
        } else if yellow.fault_blink {
            p_yellow < FAULT_ON_MS
        } else {
            compose_with_pulses(yellow.base_on, yellow.pulses, p_yellow)
        };
        apply_led(&mut pins.yellow, yellow_on);

        // GREEN (with async pulse overlay)
        let mut green_on = compose_with_pulses(green.base_on, green.pulses, p_green);
        if let Some(until) = green_pulse_until {
            if now < until { green_on = !green_on; } else { green_pulse_until = None; }
        }
        apply_led(&mut pins.green, green_on);

        // BLUE
        let blue_on = if blue.dropout_blink {
            (p_blue / 500) % 2 == 0
        } else if blue.fault_blink {
            p_blue < FAULT_ON_MS
        } else {
            compose_with_pulses(blue.base_on, blue.pulses, p_blue)
        };
        apply_led(&mut pins.blue, blue_on);

        // Tick
        Timer::after(Duration::from_millis(10)).await;
    }
}
