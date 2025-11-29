use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const CHG_CONFIG_REG: u8 = 0x31;

const CTRL_BIT_AUTO: u8 = 1 << 0;
const CTRL_BIT_MANUAL_ENABLE: u8 = 1 << 1;
const CTRL_SPEED_SHIFT: u8 = 2;
const CTRL_SPEED_MASK: u8 = 0b11 << CTRL_SPEED_SHIFT;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ChargeControlSnapshot {
    pub auto_enabled: bool,
    pub manual_enable: bool,
    pub speed: ChargeSpeedSetting,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChargeSpeedSetting {
    Slow = 0,
    Rate0p2C = 1,
    Rate0p3C = 2,
    Rate0p4C = 3,
}

impl ChargeSpeedSetting {
    pub fn from_u8(value: u8) -> Self {
        match value & 0x03 {
            0 => Self::Slow,
            1 => Self::Rate0p2C,
            2 => Self::Rate0p3C,
            _ => Self::Rate0p4C,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl Default for ChargeSpeedSetting {
    fn default() -> Self {
        Self::Slow
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SpeedLimits {
    pub ibus_limit_ma: u16,
    pub ibat_limit_ma: u16,
}

impl SpeedLimits {
    pub const fn new(ibus_limit_ma: u16, ibat_limit_ma: u16) -> Self {
        Self {
            ibus_limit_ma,
            ibat_limit_ma,
        }
    }
}

// Slow tier IBAT limit is chosen to be as low as reasonably possible while still
// respecting the SC8815 datasheet constraint that IBAT_LIM >= 300 mA.
// With the current hardware (RS2 = 10 mΩ, IBAT_RATIO = 12x; see sc8815_task.rs),
// the first valid IBAT_LIM_SET code that satisfies this is ≈328 mA, which we
// expose here as 330 mA to keep a small safety margin and a round number.
const SPEED_LIMITS: [SpeedLimits; 4] = [
    SpeedLimits::new(1000, 330),
    SpeedLimits::new(1400, 800),
    SpeedLimits::new(1900, 1200),
    SpeedLimits::new(2400, 1600),
];

pub fn limits_for(speed: ChargeSpeedSetting) -> SpeedLimits {
    SPEED_LIMITS[speed as usize]
}

static AUTO_ENABLED: AtomicBool = AtomicBool::new(true);
static MANUAL_REQUEST: AtomicBool = AtomicBool::new(true);
static OBSERVED_ENABLE: AtomicBool = AtomicBool::new(false);
static SPEED_SETTING: AtomicU8 = AtomicU8::new(ChargeSpeedSetting::Slow as u8);

pub fn reset_state() {
    AUTO_ENABLED.store(true, Ordering::Relaxed);
    MANUAL_REQUEST.store(true, Ordering::Relaxed);
    OBSERVED_ENABLE.store(false, Ordering::Relaxed);
    SPEED_SETTING.store(ChargeSpeedSetting::Slow as u8, Ordering::Relaxed);
}

fn update_control(raw: u8) {
    let auto_prev = AUTO_ENABLED.load(Ordering::Relaxed);
    let auto_new = (raw & CTRL_BIT_AUTO) != 0;
    if auto_prev && !auto_new {
        let observed = OBSERVED_ENABLE.load(Ordering::Relaxed);
        MANUAL_REQUEST.store(observed, Ordering::Relaxed);
    }
    AUTO_ENABLED.store(auto_new, Ordering::Relaxed);
    if !auto_new {
        let manual = (raw & CTRL_BIT_MANUAL_ENABLE) != 0;
        MANUAL_REQUEST.store(manual, Ordering::Relaxed);
    }
}

fn update_speed_from_config(raw: u8) {
    let tier = ChargeSpeedSetting::from_u8((raw & CTRL_SPEED_MASK) >> CTRL_SPEED_SHIFT);
    SPEED_SETTING.store(tier.as_u8(), Ordering::Relaxed);
}

fn update_config(raw: u8) {
    update_control(raw);
    update_speed_from_config(raw);
}

fn current_control_bits() -> u8 {
    let auto = AUTO_ENABLED.load(Ordering::Relaxed);
    let manual_requested = MANUAL_REQUEST.load(Ordering::Relaxed);
    let observed = OBSERVED_ENABLE.load(Ordering::Relaxed);
    let manual_bit = if auto { observed } else { manual_requested };
    let mut value = 0u8;
    if auto {
        value |= CTRL_BIT_AUTO;
    }
    if manual_bit {
        value |= CTRL_BIT_MANUAL_ENABLE;
    }
    value | ((SPEED_SETTING.load(Ordering::Relaxed) & 0x03) << CTRL_SPEED_SHIFT)
}

pub fn config_register_value() -> u8 {
    current_control_bits()
}

pub fn write_config(raw: u8) -> u8 {
    update_config(raw);
    current_control_bits()
}

pub fn update_observed_enable(active: bool) {
    OBSERVED_ENABLE.store(active, Ordering::Relaxed);
}

pub fn snapshot() -> ChargeControlSnapshot {
    ChargeControlSnapshot {
        auto_enabled: AUTO_ENABLED.load(Ordering::Relaxed),
        manual_enable: MANUAL_REQUEST.load(Ordering::Relaxed),
        speed: ChargeSpeedSetting::from_u8(SPEED_SETTING.load(Ordering::Relaxed)),
    }
}

pub fn current_speed() -> ChargeSpeedSetting {
    ChargeSpeedSetting::from_u8(SPEED_SETTING.load(Ordering::Relaxed))
}

#[allow(dead_code)]
pub fn manual_request() -> bool {
    MANUAL_REQUEST.load(Ordering::Relaxed)
}

pub fn manual_override_active() -> bool {
    !AUTO_ENABLED.load(Ordering::Relaxed) && MANUAL_REQUEST.load(Ordering::Relaxed)
}
