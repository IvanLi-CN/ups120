use defmt::*;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::Pull;

#[embassy_executor::task]
pub async fn bq_alert_task(mut input: ExtiInput<'static>) {
    info!("exti: BQ ALERT task start");
    loop {
        // Any edge on ALERT wakes the CPU; bump sleep manager so logs are coherent.
        let _ = input.wait_for_any_edge().await;
        crate::sleep_manager::bump("bq_alert");
        info!("exti: BQ ALERT edge");
    }
}

