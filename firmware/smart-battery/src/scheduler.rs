use core::sync::atomic::{AtomicBool, Ordering};

use defmt::*;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
// Single global Signal to avoid double-init

use crate::global_state::BatteryGlobalState;
use crate::shared::GlobalStateSubscriber;
use crate::sleep_manager::{self, BusyGuard};

static QUIESCE: AtomicBool = AtomicBool::new(false);
static NOTIFY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub fn is_quiesced() -> bool {
    QUIESCE.load(Ordering::Relaxed)
}

pub async fn wait_until_active() {
    if !is_quiesced() {
        return;
    }
    loop {
        NOTIFY.wait().await;
        if !is_quiesced() {
            return;
        }
    }
}

#[embassy_executor::task]
pub async fn power_scheduler_task(mut gs_sub: GlobalStateSubscriber<'static>) {
    debug!("sched:start");
    let mut hold: Option<BusyGuard> = None;
    let mut last: Option<BatteryGlobalState> = None;
    loop {
        let s = gs_sub.next_message_pure().await;
        let _prev = last;
        last = Some(s);

        // Adapter policy
        if s.ac_present {
            QUIESCE.store(false, Ordering::Relaxed);
            if hold.is_none() {
                hold = Some(sleep_manager::hold("ac_present"));
                debug!("sched:ac=1 busy=1");
            }
            NOTIFY.signal(());
        } else {
            QUIESCE.store(true, Ordering::Relaxed);
            if hold.take().is_some() {
                debug!("sched:ac=0 busy=0");
            }
            // 不再发送唤醒信号；保持静默以避免休眠后立刻被误唤醒
        }
    }
}
