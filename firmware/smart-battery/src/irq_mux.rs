use embassy_futures::select::{Either, select};
use embassy_stm32::exti::ExtiInput;

#[embassy_executor::task]
pub async fn irq_mux_task(mut sc_int: ExtiInput<'static>, mut bq_alert: ExtiInput<'static>) {
    loop {
        match select(
            sc_int.wait_for_falling_edge(),
            bq_alert.wait_for_rising_edge(),
        )
        .await
        {
            Either::First(_) => {
                crate::sc8815_task::set_sc_int_pending();
            }
            Either::Second(_) => {
                crate::bq76920_task::set_bq_alert_pending();
            }
        }
    }
}
