use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use ina226::INA226;

use crate::shared::Ina226MeasurementsPublisher;

#[embassy_executor::task]
pub async fn ina226_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    ina226_measurements_publisher: Ina226MeasurementsPublisher<'static>,
) {
    info!("INA226 task started.");
    // Create temporary I2cDevice instance for each operation
    let mut ina226 = INA226::new(i2c_bus, address); // Use the passed address

    // Resistance 10mOhm, Max 10A
    if let Err(_) = ina226.callibrate(0.01, 10.0).await {
        defmt::error!("INA226: Failed to calibrate INA226");
    }

    loop {
        // --- Reading INA226 Data ---
        let voltage_mv_f64 = ina226.bus_voltage_millivolts().await.unwrap_or(0.0);
        let current_ma_f64 = ina226
            .current_amps()
            .await
            .unwrap_or(None)
            .map_or(0.0, |c| c * 1000.0);
        let power_mw_f64 = ina226
            .power_watts()
            .await
            .unwrap_or(None)
            .map_or(0.0, |p| p * 1000.0);

        let ina226_measurements = crate::data_types::Ina226Measurements {
            voltage: voltage_mv_f64 as f32,
            current: current_ma_f64 as f32,
            power: power_mw_f64 as f32,
        };
        ina226_measurements_publisher.publish_immediate(ina226_measurements);
        info!(
            "INA226 Measurements: Voltage: {}mV, Current: {}mA, Power: {}mW",
            ina226_measurements.voltage, ina226_measurements.current, ina226_measurements.power
        );

        Timer::after(Duration::from_secs(1)).await; // Adjust delay as needed
    }
}
