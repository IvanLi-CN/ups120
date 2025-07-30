//! 数据类型定义模块
//! 
//! 包含UPS系统中各个设备的测量数据和告警信息的数据结构定义

use bq769x0_async_rs::data_types::{Bq76920Measurements as Bq76920CoreMeasurements, SystemStatus};
use sc8815::{AdcMeasurements as Sc8815AdcMeasurements, SC8815Status};

/// BQ76920 测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bq76920Measurements<const N: usize> {
    pub core_measurements: Bq76920CoreMeasurements<N>,
}

impl<const N: usize> Default for Bq76920Measurements<N> {
    fn default() -> Self {
        Self {
            core_measurements: Bq76920CoreMeasurements::default(),
        }
    }
}

/// BQ76920 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}

impl Default for Bq76920Alerts {
    fn default() -> Self {
        Self {
            system_status: SystemStatus::default(),
        }
    }
}

/// INA226 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
pub struct Ina226Measurements {
    pub voltage: f32,
    pub current: f32,
    pub power: f32,
}

impl Default for Ina226Measurements {
    fn default() -> Self {
        Self {
            voltage: 0.0,
            current: 0.0,
            power: 0.0,
        }
    }
}

/// SC8815 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
pub struct Sc8815Measurements {
    pub adc_measurements: Sc8815AdcMeasurements,
}

impl Default for Sc8815Measurements {
    fn default() -> Self {
        Self {
            adc_measurements: Sc8815AdcMeasurements::default(),
        }
    }
}

/// SC8815 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Sc8815Alerts {
    pub device_status: SC8815Status,
}

impl Default for Sc8815Alerts {
    fn default() -> Self {
        Self {
            device_status: SC8815Status::default(),
        }
    }
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AllMeasurements<const N: usize> {
    pub sc8815: Sc8815Measurements,
    pub bq76920: Bq76920Measurements<N>,
    pub sc8815_alerts: Sc8815Alerts,
    pub bq76920_alerts: Bq76920Alerts,
}

impl<const N: usize> Default for AllMeasurements<N> {
    fn default() -> Self {
        Self {
            sc8815: Sc8815Measurements::default(),
            bq76920: Bq76920Measurements::default(),
            sc8815_alerts: Sc8815Alerts::default(),
            bq76920_alerts: Bq76920Alerts::default(),
        }
    }
}

// Implementation block for AllMeasurements
impl<const N: usize> AllMeasurements<N> {
    /// Converts the aggregated measurements into the flattened USB payload structure.
    /// Assumes that BQ76920 temperatures and current are already in physical units within `self.bq76920.core_measurements`.
    pub fn to_usb_payload(&self) -> AllMeasurementsUsbPayload {
        // SC8815 ADC measurements (already in mV/mA in self.sc8815.adc_measurements)
        let sc8815_adc_vbus_mv = self.sc8815.adc_measurements.vbus_mv;
        let sc8815_adc_vbat_mv = self.sc8815.adc_measurements.vbat_mv;
        let sc8815_adc_ibus_ma = self.sc8815.adc_measurements.ibus_ma;
        let sc8815_adc_ibat_ma = self.sc8815.adc_measurements.ibat_ma;
        let sc8815_adc_adin_mv = self.sc8815.adc_measurements.adin_mv;

        // BQ76920 Temperatures (already in 0.01°C in self.bq76920.core_measurements.temperatures)
        let bq76920_temps = self.bq76920.core_measurements.temperatures;
        let ts1_temp_0_01c_val = bq76920_temps.ts1;
        let ts2_temp_0_01c_val = bq76920_temps.ts2.unwrap_or(i16::MIN); // Use sentinel for None
        let ts3_temp_0_01c_val = bq76920_temps.ts3.unwrap_or(i16::MIN); // Use sentinel for None

        let bq76920_is_thermistor_flag = self.bq76920.core_measurements.is_thermistor_mode;

        AllMeasurementsUsbPayload {
            sc8815_adc_vbus_mv,
            sc8815_adc_vbat_mv,
            sc8815_adc_ibus_ma,
            sc8815_adc_ibat_ma,
            sc8815_adc_adin_mv,

            bq76920_cell1_mv: self.bq76920.core_measurements.cell_voltages.voltages[0],
            bq76920_cell2_mv: self.bq76920.core_measurements.cell_voltages.voltages[1],
            bq76920_cell3_mv: self.bq76920.core_measurements.cell_voltages.voltages[2],
            bq76920_cell4_mv: self.bq76920.core_measurements.cell_voltages.voltages[3],
            bq76920_cell5_mv: self.bq76920.core_measurements.cell_voltages.voltages[4], // Assuming N=5
            bq76920_total_voltage_mv: self.bq76920.core_measurements.total_voltage_mv,
            bq76920_ts1_temp_0_01c: ts1_temp_0_01c_val,
            bq76920_ts2_present: self.bq76920.core_measurements.temperatures.ts2.is_some() as u8,
            bq76920_ts2_temp_0_01c: ts2_temp_0_01c_val,
            bq76920_ts3_present: self.bq76920.core_measurements.temperatures.ts3.is_some() as u8,
            bq76920_ts3_temp_0_01c: ts3_temp_0_01c_val,
            bq76920_is_thermistor: bq76920_is_thermistor_flag as u8,
            bq76920_current_ma: self.bq76920.core_measurements.current_ma,
            bq76920_system_status_mask: self.bq76920.core_measurements.system_status.0.bits(),
            bq76920_mos_status_mask: self.bq76920.core_measurements.mos_status.0.bits(),

            sc8815_device_status_flags: {
                let status = &self.sc8815_alerts.device_status;
                let mut flags = 0u8;
                if status.eoc {
                    flags |= 0x01;
                }
                if status.otp_fault {
                    flags |= 0x02;
                }
                if status.vbus_short_fault {
                    flags |= 0x04;
                }
                if status.usb_load_detected {
                    flags |= 0x08;
                }
                if status.ac_adapter_connected {
                    flags |= 0x10;
                }
                flags
            },

            bq76920_alerts_system_status_mask: self.bq76920_alerts.system_status.0.bits(),
        }
    }
}

/// Payload structure for USB communication, containing flattened data from AllMeasurements.
#[derive(Debug, Copy, Clone, PartialEq, binrw::BinWrite, defmt::Format)]
pub struct AllMeasurementsUsbPayload {
    // Fields from SC8815 ADC measurements
    pub sc8815_adc_vbus_mv: u16, // VBUS voltage in mV
    pub sc8815_adc_vbat_mv: u16, // VBAT voltage in mV
    pub sc8815_adc_ibus_ma: u16, // IBUS current in mA
    pub sc8815_adc_ibat_ma: u16, // IBAT current in mA
    pub sc8815_adc_adin_mv: u16, // ADIN voltage in mV

    // Fields from Bq76920Measurements -> Bq76920CoreMeasurements<N>
    pub bq76920_cell1_mv: i32,         // Cell 1 voltage in mV
    pub bq76920_cell2_mv: i32,         // Cell 2 voltage in mV
    pub bq76920_cell3_mv: i32,         // Cell 3 voltage in mV
    pub bq76920_cell4_mv: i32,         // Cell 4 voltage in mV
    pub bq76920_cell5_mv: i32,         // Cell 5 voltage in mV (assuming N=5)
    pub bq76920_total_voltage_mv: i32, // Total voltage of the BQ76920 pack
    pub bq76920_ts1_temp_0_01c: i16,   // TS1 temperature in 0.01°C
    pub bq76920_ts2_present: u8,       // TS2 present flag
    pub bq76920_ts2_temp_0_01c: i16,   // TS2 temperature in 0.01°C (use i16::MIN if not present)
    pub bq76920_ts3_present: u8,       // TS3 present flag
    pub bq76920_ts3_temp_0_01c: i16,   // TS3 temperature in 0.01°C (use i16::MIN if not present)
    pub bq76920_is_thermistor: u8,     // Thermistor mode flag
    pub bq76920_current_ma: i32,       // Current in mA

    pub bq76920_system_status_mask: u8, // BQ76920 system status bits
    pub bq76920_mos_status_mask: u8,    // BQ76920 MOS status bits

    // Fields from SC8815 device status
    pub sc8815_device_status_flags: u8, // SC8815 device status flags

    // Fields from Bq76920Alerts
    pub bq76920_alerts_system_status_mask: u8, // BQ76920 alerts system status bits
}
