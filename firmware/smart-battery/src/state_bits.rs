use core::sync::atomic::{AtomicU8, AtomicU16, Ordering};
use critical_section::with;

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
}

static STATE_FLAGS: AtomicU16 = AtomicU16::new(0);
static BLUE_CODE: AtomicU8 = AtomicU8::new(0);

#[inline]
pub fn update_flags(mask: u16, value_bits: u16) {
    with(|_| {
        let current = STATE_FLAGS.load(Ordering::Relaxed);
        let next = (current & !mask) | (value_bits & mask);
        STATE_FLAGS.store(next, Ordering::Relaxed);
    });
}

#[inline]
pub fn flags() -> u16 {
    STATE_FLAGS.load(Ordering::Relaxed)
}

#[inline]
pub fn set_blue_code(code: u8) {
    BLUE_CODE.store(code, Ordering::Relaxed);
}

#[inline]
pub fn blue_code() -> u8 {
    BLUE_CODE.load(Ordering::Relaxed)
}

#[derive(Copy, Clone)]
pub struct Snapshot {
    pub flags: u16,
    pub blue_code: u8,
}

#[inline]
pub fn snapshot() -> Snapshot {
    Snapshot {
        flags: flags(),
        blue_code: blue_code(),
    }
}
