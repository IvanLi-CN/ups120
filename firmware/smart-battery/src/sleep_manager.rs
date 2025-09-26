use defmt::*;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use portable_atomic::{AtomicBool, AtomicU32};
use static_cell::StaticCell;

static ACTIVE_COUNT: AtomicU32 = AtomicU32::new(0);
static SLEEPING: AtomicBool = AtomicBool::new(false);
static mut LAST_ACTIVITY_MS: u64 = 0;

const SLEEP_REENTER_IDLE_MS: u64 = 300;

static NOTIFY_CELL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
fn notify() -> &'static Signal<CriticalSectionRawMutex, ()> { NOTIFY_CELL.init(Signal::new()) }

pub struct BusyGuard;

impl BusyGuard {
    fn new() -> Self { BusyGuard }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        ACTIVE_COUNT.fetch_sub(1, portable_atomic::Ordering::Relaxed);
        notify().signal(());
        bump("drop");
    }
}

pub fn hold(_who: &str) -> BusyGuard {
    ACTIVE_COUNT.fetch_add(1, portable_atomic::Ordering::Relaxed);
    notify().signal(());
    BusyGuard::new()
}

pub fn bump(_who: &str) {
    unsafe { LAST_ACTIVITY_MS = Instant::now().as_millis(); }
    notify().signal(());
}

#[embassy_executor::task]
pub async fn sleep_task() {
    warn!("sleep:start");
    bump("start");
    let n = notify();
    loop {
        // Wait until system is idle for SLEEP_REENTER_IDLE_MS, or activity occurs.
        if ACTIVE_COUNT.load(portable_atomic::Ordering::Relaxed) > 0 {
            n.wait().await;
            continue;
        }
        let now = Instant::now().as_millis();
        let last = unsafe { LAST_ACTIVITY_MS };
        let elapsed = now.saturating_sub(last);
        if elapsed >= SLEEP_REENTER_IDLE_MS {
            if !SLEEPING.swap(true, portable_atomic::Ordering::Relaxed) {
                warn!("sleep+ {}ms", elapsed);
            }
            // Stay asleep until any activity (no timers while sleeping)
            n.wait().await;
            if SLEEPING.swap(false, portable_atomic::Ordering::Relaxed) {
                let active = ACTIVE_COUNT.load(portable_atomic::Ordering::Relaxed);
                let idle_ms = Instant::now().as_millis().saturating_sub(unsafe { LAST_ACTIVITY_MS });
                warn!("sleep- a={} i={}ms", active, idle_ms);
            }
            continue;
        }
        // Need to wait remaining time; if activity happens during this window, the
        // elapsed check after wake will prevent entering sleep.
        let wait_ms = (SLEEP_REENTER_IDLE_MS - elapsed) as u64;
        Timer::after(Duration::from_millis(wait_ms)).await;
    }
}

pub fn is_sleeping() -> bool { SLEEPING.load(portable_atomic::Ordering::Relaxed) }
