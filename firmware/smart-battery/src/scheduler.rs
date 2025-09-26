use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::*;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_sync::pubsub::Subscriber;
use static_cell::StaticCell;

use crate::global_state::BatteryGlobalState;
use crate::shared::GlobalStateSubscriber;
use crate::sleep_manager::{self, BusyGuard};

static QUIESCE: AtomicBool = AtomicBool::new(false);
static NOTIFY_CELL: StaticCell<Signal<CriticalSectionRawMutex, ()>> = StaticCell::new();
fn notify() -> &'static Signal<CriticalSectionRawMutex, ()> { NOTIFY_CELL.init(Signal::new()) }

pub fn is_quiesced() -> bool { QUIESCE.load(Ordering::Relaxed) }

pub async fn wait_until_active() {
    if !is_quiesced() { return; }
    loop {
        notify().wait().await;
        if !is_quiesced() { return; }
    }
}


#[embassy_executor::task]
pub async fn power_scheduler_task(mut gs_sub: GlobalStateSubscriber<'static>) {
    debug!("sched:start");
    let mut hold: Option<BusyGuard> = None;
    let mut last: Option<BatteryGlobalState> = None;
    loop {
        let s = gs_sub.next_message_pure().await;
        let prev = last;
        last = Some(s);

        // Adapter policy
        if s.ac_present {
            QUIESCE.store(false, Ordering::Relaxed);
            if hold.is_none() {
                hold = Some(sleep_manager::hold("ac_present"));
                debug!("sched:ac=1 busy=1");
            }
            notify().signal(());
        } else {
            QUIESCE.store(true, Ordering::Relaxed);
            if hold.take().is_some() {
                debug!("sched:ac=0 busy=0");
            }
            // Nudge timestamp so sleep manager can consider entering sleep sooner
            // (no busy token held, bumps not strictly required but helps logs ordering)
            sleep_manager::bump("ac_absent");
            notify().signal(());
        }
    }
}
