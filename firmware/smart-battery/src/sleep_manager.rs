use defmt::*;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use portable_atomic::{AtomicBool, AtomicU32};

static ACTIVE_COUNT: AtomicU32 = AtomicU32::new(0);
static SLEEPING: AtomicBool = AtomicBool::new(false);
static mut LAST_ACTIVITY_MS: u64 = 0;
static mut FORBID_SLEEP_UNTIL_MS: u64 = 0; // wake-holdoff deadline (ms)
static WAKE_LATCH: AtomicBool = AtomicBool::new(false); // set by holders at work start
static WAKE_CAUSE_PRINTED: AtomicBool = AtomicBool::new(false); // throttle cause log per sleep cycle

const SLEEP_REENTER_IDLE_MS: u64 = 300;
const WAKE_HOLDOFF_MS: u64 = 8000; // Keep awake >=8s after activity

static NOTIFY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub struct BusyGuard;

impl BusyGuard {
    fn new() -> Self {
        BusyGuard
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        ACTIVE_COUNT.fetch_sub(1, portable_atomic::Ordering::Relaxed);
        NOTIFY.signal(());
        bump("drop");
    }
}

pub fn hold(_who: &str) -> BusyGuard {
    ACTIVE_COUNT.fetch_add(1, portable_atomic::Ordering::Relaxed);
    unsafe {
        LAST_ACTIVITY_MS = Instant::now().as_millis();
    }
    unsafe {
        FORBID_SLEEP_UNTIL_MS = LAST_ACTIVITY_MS.saturating_add(WAKE_HOLDOFF_MS);
    }
    let was = WAKE_LATCH.swap(true, portable_atomic::Ordering::Relaxed);
    if !was && !WAKE_CAUSE_PRINTED.swap(true, portable_atomic::Ordering::Relaxed) {
        info!("wake: cause={}()", _who);
    }
    NOTIFY.signal(());
    BusyGuard::new()
}

pub fn bump(_who: &str) {
    unsafe {
        LAST_ACTIVITY_MS = Instant::now().as_millis();
    }
    NOTIFY.signal(());
}

#[embassy_executor::task]
pub async fn sleep_task() {
    warn!("sleep: start (mode=SLEEP)");
    bump("start");
    let n = &NOTIFY;
    loop {
        if ACTIVE_COUNT.load(portable_atomic::Ordering::Relaxed) > 0 {
            n.wait().await;
            continue;
        }
        let now = Instant::now().as_millis();
        let last = unsafe { LAST_ACTIVITY_MS };
        let elapsed = now.saturating_sub(last);
        let forbid_until = unsafe { FORBID_SLEEP_UNTIL_MS };
        if now < forbid_until {
            let remain = forbid_until - now;
            let need = (SLEEP_REENTER_IDLE_MS.saturating_sub(elapsed)) as u64;
            let wait_ms = remain.max(need);
            Timer::after(Duration::from_millis(wait_ms)).await;
            continue;
        }
        if elapsed >= SLEEP_REENTER_IDLE_MS {
            if !SLEEPING.swap(true, portable_atomic::Ordering::Relaxed) {
                warn!("sleep: enter (idle_ms={})", elapsed);
                WAKE_LATCH.store(false, portable_atomic::Ordering::Relaxed);
                WAKE_CAUSE_PRINTED.store(false, portable_atomic::Ordering::Relaxed);
            }
            loop {
                n.wait().await;
                let active = ACTIVE_COUNT.load(portable_atomic::Ordering::Relaxed);
                let latched = WAKE_LATCH.swap(false, portable_atomic::Ordering::Relaxed);
                if active == 0 && !latched {
                    continue;
                }
                if SLEEPING.swap(false, portable_atomic::Ordering::Relaxed) {
                    let now2 = Instant::now().as_millis();
                    let idle_ms = now2.saturating_sub(unsafe { LAST_ACTIVITY_MS });
                    warn!(
                        "sleep: exit (active_count={} idle_ms={} latched={})",
                        active, idle_ms, latched
                    );
                    unsafe {
                        FORBID_SLEEP_UNTIL_MS = now2.saturating_add(WAKE_HOLDOFF_MS);
                    }
                }
                break;
            }
            continue;
        }
        let wait_ms = (SLEEP_REENTER_IDLE_MS - elapsed) as u64;
        Timer::after(Duration::from_millis(wait_ms)).await;
    }
}

#[allow(dead_code)]
pub fn is_sleeping() -> bool {
    SLEEPING.load(portable_atomic::Ordering::Relaxed)
}
