use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::*;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::Subscriber;

use crate::global_state::BatteryGlobalState;
use crate::shared::GlobalStateSubscriber;
use crate::sleep_manager::{self, BusyGuard};

static QUIESCE: AtomicBool = AtomicBool::new(false);

pub fn is_quiesced() -> bool { QUIESCE.load(Ordering::Relaxed) }

#[embassy_executor::task]
pub async fn power_scheduler_task(mut gs_sub: GlobalStateSubscriber<'static>) {
    info!("power_scheduler: start");
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
                info!("sched: AC present → stay awake (busy token on)\n");
            }
        } else {
            QUIESCE.store(true, Ordering::Relaxed);
            if hold.take().is_some() {
                info!("sched: AC absent → allow sleep (busy token off)\n");
            }
            // Nudge timestamp so sleep manager can consider entering sleep sooner
            // (no busy token held, bumps not strictly required but helps logs ordering)
            sleep_manager::bump("ac_absent");
        }
    }
}

