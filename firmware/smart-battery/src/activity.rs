use core::sync::atomic::{AtomicBool, Ordering};

// Simple cross-task activity flags.
// Green LED emits an async pulse whenever this flag is observed true.
pub static I2C1_ACTIVITY_PULSE: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn poke_i2c1_activity() {
    I2C1_ACTIVITY_PULSE.store(true, Ordering::Relaxed);
}
