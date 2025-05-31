use defmt::Format;

use bq769x0_async_rs::data_types::{Bq76920Measurements as Bq76920CoreMeasurements, SystemStatus};
use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]

pub struct Bq25730Measurements {
    pub adc_measurements: AdcMeasurements,
    // 添加其他非告警相关的测量数据字段（如果需要）
}

/// BQ25730 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct Bq25730Alerts {
    pub charger_status: ChargerStatus,
    pub prochot_status: ProchotStatus,
}

/// BQ76920 测量数据
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct Bq76920Measurements<const N: usize> {
    pub core_measurements: Bq76920CoreMeasurements<N>,
}

/// BQ76920 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}
/// INA226 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]

pub struct Ina226Measurements {
    pub voltage: f32,
    pub current: f32,
    pub power: f32, // 假设需要功率，如果不需要可以调整
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct AllMeasurements<const N: usize> {
    pub bq25730: Bq25730Measurements,
    pub bq76920: Bq76920Measurements<N>,
    pub ina226: Ina226Measurements,
    pub bq25730_alerts: Bq25730Alerts,
    pub bq76920_alerts: Bq76920Alerts,
}

/// Payload structure for USB communication, containing flattened data from AllMeasurements.
#[derive(Debug, Copy, Clone, PartialEq, binrw::BinWrite, defmt::Format)] // Removed binrw::BinRead
pub struct AllMeasurementsUsbPayload {
    // Fields from Bq25730Measurements -> AdcMeasurements
    pub bq25730_adc_vbat_raw: u16, // Raw u16 value after AdcVbat.to_u16()
    pub bq25730_adc_vsys_raw: u16, // Raw u16 value after AdcVsys.to_u16()
    pub bq25730_adc_ichg_raw: u16, // Raw u16 value after AdcIchg.to_u16()
    pub bq25730_adc_idchg_raw: u16, // Raw u16 value after AdcIdchg.to_u16()
    pub bq25730_adc_iin_raw: u16, // Raw u16 value after AdcIin.to_u16() (represents milliamps part)
    pub bq25730_adc_psys_raw: u16, // Raw u16 value after AdcPsys.to_u16()
    pub bq25730_adc_vbus_raw: u16, // Raw u16 value after AdcVbus.to_u16()
    pub bq25730_adc_cmpin_raw: u16, // Raw u16 value after AdcCmpin.to_u16()

    // Fields from Bq76920Measurements -> Bq76920CoreMeasurements<5>
    pub bq76920_cell1_mv: i32,
    pub bq76920_cell2_mv: i32,
    pub bq76920_cell3_mv: i32,
    pub bq76920_cell4_mv: i32,
    pub bq76920_cell5_mv: i32,
    pub bq76920_ts1_raw_adc: u16,
    pub bq76920_ts2_present: u8,   // 1 if Some, 0 if None
    pub bq76920_ts2_raw_adc: u16,  // value or 0 if not present
    pub bq76920_ts3_present: u8,   // 1 if Some, 0 if None
    pub bq76920_ts3_raw_adc: u16,  // value or 0 if not present
    pub bq76920_is_thermistor: u8, // 0 or 1
    pub bq76920_current_ma: i32,
    pub bq76920_system_status_bits: u8,
    pub bq76920_mos_status_bits: u8,

    // Fields from Ina226Measurements
    pub ina226_voltage_f32: f32,
    pub ina226_current_f32: f32,
    pub ina226_power_f32: f32,

    // Fields from Bq25730Alerts
    pub bq25730_charger_status_raw_u16: u16, // Raw u16 from ChargerStatus.to_u16()
    pub bq25730_prochot_status_raw_u16: u16, // Raw u16 from ProchotStatus.to_u16()

    // Fields from Bq76920Alerts
    pub bq76920_alerts_system_status_bits: u8, // Raw u8 from SystemStatus.0.bits()
}

impl<const N: usize> Format for AllMeasurements<N> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "AllMeasurements {{ bq25730_measurements: {}, bq76920_measurements: {{ cell_voltages: [",
            self.bq25730 // This is Bq25730Measurements, which has its own Format impl
        );
        for i in 0..N {
            defmt::write!(
                fmt,
                "{:?}, ",
                self.bq76920.core_measurements.cell_voltages.voltages[i]
            );
        }
        defmt::write!(
            fmt,
            "], temperatures: {{ ts1_0_01C: {:?}, ts2_0_01C: {:?}, ts3_0_01C: {:?}, is_thermistor: {} }}, current_mA: {}, system_status: {{ cc_ready: {}, device_xready: {}, ovrd_alert: {}, uv: {}, ov: {}, scd: {}, ocd: {} }}, mos_status: {{ charge_on: {}, discharge_on: {} }} }}, ina226_measurements: {}, bq25730_alerts: {{ charger_status: {{ status: {=u8:b}, fault: {=u8:b} }}, prochot_status: {{ msb: {=u8:b}, lsb: {=u8:b}, width: {} }} }}, bq76920_alerts: {{ system_status: {{ cc_ready: {}, device_xready: {}, ovrd_alert: {}, uv: {}, ov: {}, scd: {}, ocd: {} }} }} }}",
            {
                let temp_data = self
                    .bq76920
                    .core_measurements
                    .temperatures
                    .into_temperature_data(None);
                temp_data.map(|td| td.ts1).ok()
            },
            {
                let temp_data = self
                    .bq76920
                    .core_measurements
                    .temperatures
                    .into_temperature_data(None);
                temp_data.map(|td| td.ts2).unwrap_or(None)
            },
            {
                let temp_data = self
                    .bq76920
                    .core_measurements
                    .temperatures
                    .into_temperature_data(None);
                temp_data.map(|td| td.ts3).unwrap_or(None)
            },
            self.bq76920.core_measurements.temperatures.is_thermistor,
            self.bq76920.core_measurements.current,
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
            self.ina226, // This is Ina226Measurements, which has its own Format impl
            self.bq25730_alerts.charger_status.status_flags.bits(),
            self.bq25730_alerts.charger_status.fault_flags.bits(),
            self.bq25730_alerts.prochot_status.msb_flags.bits(),
            self.bq25730_alerts.prochot_status.lsb_flags.bits(),
            self.bq25730_alerts.prochot_status.prochot_width,
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::UV),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OV),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::SCD),
            self.bq76920_alerts
                .system_status
                .0
                .contains(bq769x0_async_rs::registers::SysStatFlags::OCD)
        );
    }
}
