use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use ina226::INA226;

use crate::shared::Ina226MeasurementsPublisher;
use crate::data_types::Ina226Measurements;

/// INA226任务 - 监控输入电源功率
/// 
/// 此任务负责：
/// - 监测输入电源的电压、电流和功率
/// - 提供系统总功耗数据
/// - 为功率管理提供关键数据
#[embassy_executor::task]
pub async fn ina226_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_rp::peripherals::I2C0, embassy_rp::i2c::Async>>,
    address: u8,
    ina226_measurements_publisher: Ina226MeasurementsPublisher<'static>,
) {
    info!("INA226 task started - monitoring input power");
    
    // Create INA226 driver instance
    let mut ina226 = INA226::new(i2c_bus, address);

    // Calibrate INA226 for power measurement
    // Resistance: 10mOhm, Max Current: 10A
    if let Err(_) = ina226.callibrate(0.01, 10.0).await {
        error!("INA226: Failed to calibrate - using default settings");
    } else {
        info!("INA226: Calibrated successfully (10mΩ, 10A max)");
    }

    loop {
        // Read INA226 measurements
        let voltage_v = ina226.bus_voltage_millivolts().await.unwrap_or(0.0) / 1000.0;
        let current_a = ina226
            .current_amps()
            .await
            .unwrap_or(None)
            .unwrap_or(0.0);
        let power_w = ina226
            .power_watts()
            .await
            .unwrap_or(None)
            .unwrap_or(0.0);

        let ina226_measurements = Ina226Measurements {
            voltage: voltage_v as f32,
            current: current_a as f32,
            power: power_w as f32,
        };

        // Publish measurements
        ina226_measurements_publisher.publish_immediate(ina226_measurements);
        
        info!(
            "[INA226] Input Power: {}V, {}A, {}W",
            ina226_measurements.voltage, 
            ina226_measurements.current, 
            ina226_measurements.power
        );

        // Wait 1 second before next measurement
        Timer::after(Duration::from_secs(1)).await;
    }
}
