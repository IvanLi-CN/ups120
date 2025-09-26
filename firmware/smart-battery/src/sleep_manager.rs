use defmt::*;
use embassy_time::{Duration, Instant, Timer};
use portable_atomic::{AtomicBool, AtomicU32};

static ACTIVE_COUNT: AtomicU32 = AtomicU32::new(0);
static SLEEPING: AtomicBool = AtomicBool::new(false);
static mut LAST_ACTIVITY_MS: u64 = 0;

const SLEEP_REENTER_IDLE_MS: u64 = 300;

pub struct BusyGuard;

impl BusyGuard {
    fn new() -> Self { BusyGuard }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        ACTIVE_COUNT.fetch_sub(1, portable_atomic::Ordering::Relaxed);
        bump("drop");
    }
}

pub fn hold(_who: &str) -> BusyGuard {
    ACTIVE_COUNT.fetch_add(1, portable_atomic::Ordering::Relaxed);
    BusyGuard::new()
}

pub fn bump(_who: &str) {
    unsafe { LAST_ACTIVITY_MS = Instant::now().as_millis(); }
}

#[embassy_executor::task]
pub async fn sleep_task() {
    info!("sleep_manager: start (SLEEP mode)");
    bump("start");
    loop {
        Timer::after(Duration::from_millis(50)).await;
        let now = Instant::now().as_millis();
        let last = unsafe { LAST_ACTIVITY_MS };
        let idle_ms = now.saturating_sub(last);
        let active = ACTIVE_COUNT.load(portable_atomic::Ordering::Relaxed);
        let sleeping = SLEEPING.load(portable_atomic::Ordering::Relaxed);

        if active == 0 && idle_ms >= SLEEP_REENTER_IDLE_MS {
            if !sleeping {
                SLEEPING.store(true, portable_atomic::Ordering::Relaxed);
                info!("sleep_enter (idle_ms={} >= {}ms)", idle_ms, SLEEP_REENTER_IDLE_MS);
            }
        } else {
            if sleeping {
                SLEEPING.store(false, portable_atomic::Ordering::Relaxed);
                info!("sleep_exit (active={} idle_ms={})", active, idle_ms);
            }
        }
    }
}

pub fn is_sleeping() -> bool { SLEEPING.load(portable_atomic::Ordering::Relaxed) }

