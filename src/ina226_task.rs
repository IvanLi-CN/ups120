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

    loop {
        // --- Reading INA226 Data ---
        let voltage_mv_f64 = ina226.bus_voltage_millivolts().await.unwrap_or(0.0);
        let current_amps_f64_opt = ina226.current_amps().await.unwrap_or(None);
        let current_ma_f64 = current_amps_f64_opt.map_or(0.0, |c| c * 1000.0); // Convert to mA

        let power_mw_f64 = match ina226.power_watts().await { // Assuming async power_watts() exists
            Ok(Some(watts)) => watts * 1000.0, // Convert Watts to milliWatts
            Ok(None) => {
                defmt::warn!("INA226: Power reading was None");
                0.0
            }
            Err(e) => {
                defmt::error!("INA226: Failed to read power: {:?}", e);
                0.0
            }
        };

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
