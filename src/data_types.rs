use binrw::binrw;
use defmt::Format;

use bq769x0_async_rs::data_types::{Bq76920Measurements as Bq76920CoreMeasurements, SystemStatus};
use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};
use uom::si::{electric_current::milliampere, electric_potential::millivolt};

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
#[binrw]
pub struct Bq25730Measurements {
    pub adc_measurements: AdcMeasurements,
    // 添加其他非告警相关的测量数据字段（如果需要）
}

/// BQ25730 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
#[binrw]
pub struct Bq25730Alerts {
    pub charger_status: ChargerStatus,
    pub prochot_status: ProchotStatus,
}

/// BQ76920 测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
#[binrw]
pub struct Bq76920Measurements<const N: usize> {
    pub core_measurements: Bq76920CoreMeasurements<N>,
}

/// BQ76920 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
#[binrw]
pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}
/// INA226 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
#[binrw]
pub struct Ina226Measurements {
    pub voltage: f32,
    pub current: f32,
    pub power: f32, // 假设需要功率，如果不需要可以调整
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
#[binrw]
pub struct AllMeasurements<const N: usize> {
    pub bq25730: Bq25730Measurements,
    pub bq76920: Bq76920Measurements<N>,
    pub ina226: Ina226Measurements,
}

impl<const N: usize> Format for AllMeasurements<N> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "AllMeasurements {{ bq25730: {}, bq76920: {{ cell_voltages: [",
            self.bq25730
        );
        for i in 0..N {
            defmt::write!(
                fmt,
                "{:?}, ",
                self.bq76920.core_measurements.cell_voltages.voltages[i].get::<millivolt>()
            );
        }
        defmt::write!(
            fmt,
            "], temperatures: {{ ts1: {:?}, is_thermistor: {} }}, current: {}, system_status: {{ cc_ready: {}, device_xready: {}, ovrd_alert: {}, uv: {}, ov: {}, scd: {}, ocd: {} }}, mos_status: {{ charge_on: {}, discharge_on: {} }} }}, ina226: {{ voltage: {}, current: {}, power: {} }} }}",
            self.bq76920
                .core_measurements
                .temperatures
                .ts1
                .get::<uom::si::electric_potential::volt>(),
            self.bq76920.core_measurements.temperatures.is_thermistor,
            self.bq76920.core_measurements.current.get::<milliampere>(),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::UV),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OV),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::SCD),
            self.bq76920
                .core_measurements
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OCD),
            // self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP), // Removed OVR_TEMP check
            self.bq76920
                .core_measurements
                .mos_status
                .0
                .contains(bq769x0_async_rs::registers::SysCtrl2Flags::CHG_ON),
            self.bq76920
                .core_measurements
                .mos_status
                .0
                .contains(bq769x0_async_rs::registers::SysCtrl2Flags::DSG_ON),
            self.ina226.voltage,
            self.ina226.current,
            self.ina226.power
        );
    }
}
