use core::sync::atomic::{AtomicBool, Ordering};

// When true, SC8815 power stage must be stopped (PSTOP high).
static BQ_FAILSAFE_PSTOP: AtomicBool = AtomicBool::new(false);
// Use a separate AtomicU32 for SC heartbeat (ms since boot, lower 32 bits)
static SC_LAST_MS32: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static QUIESCE: AtomicBool = AtomicBool::new(false); // true=外设静默（无 AC）

// Online flags: reflect whether device is considered online after boot probe or recent comm success.
static SC_ONLINE: AtomicBool = AtomicBool::new(false);
static BQ_ONLINE: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn request_pstop() {
    BQ_FAILSAFE_PSTOP.store(true, Ordering::Relaxed);
}

#[inline]
pub fn clear_pstop() {
    BQ_FAILSAFE_PSTOP.store(false, Ordering::Relaxed);
}

#[inline]
pub fn is_pstop_requested() -> bool {
    BQ_FAILSAFE_PSTOP.load(Ordering::Relaxed)
}

#[inline]
pub fn sc_heartbeat_update(now_ms: u32) {
    SC_LAST_MS32.store(now_ms, Ordering::Relaxed);
}

// Note: `sc_last_ms` accessor removed; online tracking relies on explicit flags now.

#[inline]
pub fn set_ac_present(ac: bool) {
    // 反向记录静默标志：无 AC => QUIESCE=true
    QUIESCE.store(!ac, Ordering::Relaxed);
}

#[inline]
pub fn is_quiesced() -> bool {
    QUIESCE.load(Ordering::Relaxed)
}

#[inline]
pub fn set_sc_online(v: bool) {
    SC_ONLINE.store(v, Ordering::Relaxed);
}

#[inline]
pub fn is_sc_online() -> bool {
    SC_ONLINE.load(Ordering::Relaxed)
}

#[inline]
pub fn set_bq_online(v: bool) {
    BQ_ONLINE.store(v, Ordering::Relaxed);
}

#[inline]
pub fn is_bq_online() -> bool {
    BQ_ONLINE.load(Ordering::Relaxed)
}
