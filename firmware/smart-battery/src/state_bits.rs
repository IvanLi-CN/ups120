use core::sync::atomic::{AtomicU8, AtomicU16, Ordering};
use critical_section::with;

use crate::i2c_slave;

/// Bit layout for `STATE_FLAGS` (lower 8 bits currently used).
/// Naming kept short to minimise call‑sites and keep flash footprint low.
pub mod bits {
    pub const AC_PRESENT: u16 = 1 << 0;
    pub const CHARGING: u16 = 1 << 1;
    pub const CHG_PAUSED: u16 = 1 << 2;
    pub const PREPARING: u16 = 1 << 3;
    pub const FULL: u16 = 1 << 4;
    pub const BALANCING: u16 = 1 << 5;
    pub const FAULT_BQ: u16 = 1 << 6;
    pub const FAULT_SC: u16 = 1 << 7;
    pub const ACTIVE_SC: u16 = 1 << 8;
    pub const ACTIVE_BQ: u16 = 1 << 9;
}

/// Pause cause bits for CHG_PAUSE_CAUSE (0x32, RO)
pub mod pause_bits {
    pub const IMBALANCE: u8 = 1 << 0;
    pub const PACK_TEMP: u8 = 1 << 1;
    pub const CHG_TEMP: u8 = 1 << 2;
    pub const OVUV_OC: u8 = 1 << 3;
    pub const HOLD_OFF: u8 = 1 << 4;
    pub const ADAPTER_MISS: u8 = 1 << 5;
    pub const EOC_FULL: u8 = 1 << 6;
}

static STATE_FLAGS: AtomicU16 = AtomicU16::new(0);
static PAUSE_CAUSE: AtomicU8 = AtomicU8::new(0);
static BLUE_CODE: AtomicU8 = AtomicU8::new(0);

#[inline]
pub fn update_flags(mask: u16, value_bits: u16) {
    let next = with(|_| {
        let current = STATE_FLAGS.load(Ordering::Relaxed);
        let next = (current & !mask) | (value_bits & mask);
        STATE_FLAGS.store(next, Ordering::Relaxed);
        next
    });
    i2c_slave::update_state_snapshot(next, blue_code());
}

#[inline]
pub fn flags() -> u16 {
    STATE_FLAGS.load(Ordering::Relaxed)
}

#[inline]
pub fn update_pause_cause(bits: u8) {
    PAUSE_CAUSE.store(bits, Ordering::Relaxed);
}

#[inline]
pub fn pause_cause() -> u8 {
    PAUSE_CAUSE.load(Ordering::Relaxed)
}

#[inline]
pub fn set_blue_code(code: u8) {
    BLUE_CODE.store(code, Ordering::Relaxed);
    i2c_slave::update_state_snapshot(flags(), code);
}

#[inline]
pub fn blue_code() -> u8 {
    BLUE_CODE.load(Ordering::Relaxed)
}
