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
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}

/// INA226 测量数据 - 监控输入电源功率
/// 用于监测输入电源的总功率消耗，包括充电模块和后级用电器的功耗
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
pub struct Ina226Measurements {
    pub voltage: f32, // 输入电压 (V)
    pub current: f32, // 输入电流 (A)
    pub power: f32,   // 输入功率 (W)
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
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format, Default)]
pub struct Sc8815Measurements {
    pub adc_measurements: Sc8815AdcMeasurements,
}

/// SC8815 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct Sc8815Alerts {
    pub device_status: SC8815Status,
}

/// OTG配置参数
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub struct OtgConfiguration {
    /// 设定电压 Vs (mV)
    pub target_voltage_mv: u16,
    /// 高阈值百分比 (默认90%)
    pub high_threshold_percent: u8,
    /// 低阈值百分比 (默认70%)
    pub low_threshold_percent: u8,
    /// 电压降低值 (默认500mV)
    pub voltage_reduction_mv: u16,
    /// 输出限流 (默认1000mA)
    pub current_limit_ma: u16,
    /// OTG功能使能
    pub enabled: bool,
}

impl Default for OtgConfiguration {
    fn default() -> Self {
        Self {
            target_voltage_mv: 12000, // 12V
            high_threshold_percent: 90,
            low_threshold_percent: 70,
            voltage_reduction_mv: 500, // 0.5V
            current_limit_ma: 1000,    // 1A
            enabled: true,
        }
    }
}

/// OTG控制状态枚举
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum OtgControlState {
    /// 高电压状态 (>90% Vs) - 输出 Vs-0.5V
    HighVoltage,
    /// 低电压状态 (<70% Vs) - 输出 Vs
    LowVoltage,
    /// 正常状态 (70%-90% Vs) - 滞回控制
    Normal,
    /// 禁用状态
    Disabled,
}

/// OTG运行状态
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub struct OtgStatus {
    /// OTG是否启用
    pub enabled: bool,
    /// 当前输出电压 (mV)
    pub output_voltage_mv: u16,
    /// 当前输出电流 (mA)
    pub output_current_ma: u16,
    /// 检测到的输入电压 (mV)
    pub input_voltage_mv: u16,
    /// 控制状态
    pub control_state: OtgControlState,
    /// 最后更新时间戳 (ms)
    pub last_update_ms: u64,
}

impl Default for OtgStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            output_voltage_mv: 0,
            output_current_ma: 0,
            input_voltage_mv: 0,
            control_state: OtgControlState::Disabled,
            last_update_ms: 0,
        }
    }
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct AllMeasurements<const N: usize> {
    pub ina226: Ina226Measurements,
    pub sc8815: Sc8815Measurements,
    pub bq76920: Bq76920Measurements<N>,
    pub sc8815_alerts: Sc8815Alerts,
    pub bq76920_alerts: Bq76920Alerts,
}

// Implementation block for AllMeasurements
impl<const N: usize> AllMeasurements<N> {
    /// Converts the aggregated measurements into the flattened USB payload structure.
    /// Assumes that BQ76920 temperatures and current are already in physical units within `self.bq76920.core_measurements`.
    pub fn to_usb_payload(self, otg_status: Option<OtgStatus>) -> AllMeasurementsUsbPayload {
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
            // INA226 fields (输入电源监控)
            ina226_voltage_mv: (self.ina226.voltage * 1000.0) as u32,
            ina226_current_ma: (self.ina226.current * 1000.0) as i32,
            ina226_power_mw: (self.ina226.power * 1000.0) as u32,

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

            // OTG字段
            otg_enabled: otg_status.map(|s| s.enabled as u8).unwrap_or(0),
            otg_output_voltage_mv: otg_status.map(|s| s.output_voltage_mv).unwrap_or(0),
            otg_output_current_ma: otg_status.map(|s| s.output_current_ma).unwrap_or(0),
            otg_input_voltage_mv: otg_status.map(|s| s.input_voltage_mv).unwrap_or(0),
            otg_control_state: otg_status.map(|s| s.control_state as u8).unwrap_or(0),
        }
    }
}

/// Payload structure for USB communication, containing flattened data from AllMeasurements.
#[derive(Debug, Copy, Clone, PartialEq, binrw::BinWrite, binrw::BinRead, defmt::Format)]
pub struct AllMeasurementsUsbPayload {
    // Fields from INA226 measurements (输入电源监控)
    pub ina226_voltage_mv: u32, // 输入电压 (mV)
    pub ina226_current_ma: i32, // 输入电流 (mA)
    pub ina226_power_mw: u32,   // 输入功率 (mW)

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

    // OTG相关字段
    pub otg_enabled: u8,            // OTG使能状态
    pub otg_output_voltage_mv: u16, // OTG输出电压
    pub otg_output_current_ma: u16, // OTG输出电流
    pub otg_input_voltage_mv: u16,  // OTG检测输入电压
    pub otg_control_state: u8,      // OTG控制状态
}
