// use defmt::Format; // Removed unused import

use bq769x0_async_rs::data_types::{Bq76920Measurements as Bq76920CoreMeasurements, SystemStatus};
use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};

// use crate::shared::Bq76920RuntimeConfig; // Removed as Bq76920RuntimeConfig is no longer needed by to_usb_payload

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]

pub struct Bq25730Measurements {
    pub adc_measurements: AdcMeasurements,
    // 添加其他非告警相关的测量数据字段（如果需要）
}

impl Default for Bq25730Measurements {
    fn default() -> Self {
        Self {
            adc_measurements: AdcMeasurements::default(),
        }
    }
}

/// BQ25730 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct Bq25730Alerts {
    pub charger_status: ChargerStatus,
    pub prochot_status: ProchotStatus,
}

impl Default for Bq25730Alerts {
    fn default() -> Self {
        Self {
            charger_status: ChargerStatus::default(),
            prochot_status: ProchotStatus::default(),
        }
    }
}

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
    pub power: f32, // 假设需要功率，如果不需要可以调整
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

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)]

pub struct AllMeasurements<const N: usize> {
    pub bq25730: Bq25730Measurements,
    pub bq76920: Bq76920Measurements<N>,
    pub ina226: Ina226Measurements,
    pub bq25730_alerts: Bq25730Alerts,
    pub bq76920_alerts: Bq76920Alerts,
}

impl<const N: usize> Default for AllMeasurements<N> {
    fn default() -> Self {
        Self {
            bq25730: Bq25730Measurements::default(),
            bq76920: Bq76920Measurements::default(),
            ina226: Ina226Measurements::default(),
            bq25730_alerts: Bq25730Alerts::default(),
            bq76920_alerts: Bq76920Alerts::default(),
        }
    }
}

// Implementation block for AllMeasurements
impl<const N: usize> AllMeasurements<N> {
    /// Converts the aggregated measurements into the flattened USB payload structure.
    /// Assumes that BQ76920 temperatures and current are already in physical units within `self.bq76920.core_measurements`.
    pub fn to_usb_payload(&self) -> AllMeasurementsUsbPayload {
        // BQ25730 Voltages (already in mV in self.bq25730.adc_measurements)
        let bq25730_adc_vbat_mv = self.bq25730.adc_measurements.vbat.0;
        let bq25730_adc_vsys_mv = self.bq25730.adc_measurements.vsys.0;
        // BQ25730 Currents (already in mA in self.bq25730.adc_measurements)
        let bq25730_adc_ichg_ma = self.bq25730.adc_measurements.ichg.milliamps;
        let bq25730_adc_idchg_ma = self.bq25730.adc_measurements.idchg.milliamps;
        let bq25730_adc_iin_ma = self.bq25730.adc_measurements.iin.milliamps;
        let bq25730_adc_psys_mv = self.bq25730.adc_measurements.psys.0;
        let bq25730_adc_vbus_mv = self.bq25730.adc_measurements.vbus.0;
        let bq25730_adc_cmpin_mv = self.bq25730.adc_measurements.cmpin.0;

        // BQ76920 Temperatures (already in 0.01°C in self.bq76920.core_measurements.temperatures)
        let bq76920_temps = self.bq76920.core_measurements.temperatures;
        let ts1_temp_0_01c_val = bq76920_temps.ts1;
        let ts2_temp_0_01c_val = bq76920_temps.ts2.unwrap_or(i16::MIN); // Use sentinel for None
        let ts3_temp_0_01c_val = bq76920_temps.ts3.unwrap_or(i16::MIN); // Use sentinel for None

        // Determine if BQ76920 is using thermistors. This info might need to come from Bq76920CoreMeasurements if it's stored there post-conversion,
        // or from a runtime config if it's still dynamic at this stage.
        // For now, assuming it's not directly available in the already converted TemperatureData.
        // This field in UsbPayload might need reconsideration or a fixed value if not dynamically known here.
        // Let's check if the original RawTemperatureAdcReadings' is_thermistor is accessible or if we need to infer.
        // Since Bq76920CoreMeasurements now holds TemperatureData, we don't have direct access to the original is_thermistor flag
        // that was part of RawTemperatureAdcReadings without further changes to Bq76920CoreMeasurements.
        // For now, we'll set it based on whether NTC parameters were used (which we no longer track here).
        // This highlights a potential need to pass the `is_thermistor` flag along with `TemperatureData` if it's required by the USB payload.
        // As a simplification, if NTC parameters were used (which implies external thermistors), then is_thermistor would be true.
        // However, the conversion now happens inside the bq769x0 library.
        // The `bq769x0_async_rs::data_types::TemperatureData` does not store `is_thermistor`.
        // The `bq769x0_async_rs::lib::read_temperatures` determines this internally.
        // We need a way to get this `is_thermistor` flag.
        // One way is to add `is_thermistor` to `bq769x0_async_rs::data_types::TemperatureData`.
        // For now, let's assume it's false for simplicity, or we need to revisit the sub-module.
        // Let's assume for now the sub-module's Bq76920Measurements might be extended to include this.
        // Or, if the USB payload *really* needs to know if the *original source* was a thermistor,
        // that's a different concern than just presenting the converted temperature.
        // The current `AllMeasurementsUsbPayload` has `bq76920_is_thermistor`.
        // This implies we need this info.
        // The simplest way without further sub-module changes is to get it from `bq76920_conf` if it still exists
        // or make it part of the `Bq76920Measurements` from the task.
        // Given the current refactoring, `bq76920_conf` is being removed from this function's scope.
        // This means `Bq76920Measurements` (from `crate::data_types`) or its `core_measurements`
        // needs to provide this. The sub-module's `Bq76920Measurements` does not currently store `is_thermistor`
        // alongside the converted `TemperatureData`.
        //
        // **Decision**: For now, to make progress, I will assume `is_thermistor` needs to be sourced
        // from the `Bq76920RuntimeConfig` if it were still passed, or be part of `Bq76920Measurements`.
        // Since we are removing `Bq76920RuntimeConfig` from `to_usb_payload`, this field becomes problematic.
        //
        // Let's assume `Bq76920RuntimeConfig` is still available in `usb_task` and passed to `to_usb_payload`
        // *only* for this `is_thermistor` flag, or that `Bq76920Measurements` gets an `is_thermistor` field.
        // For now, I will keep the `bq76920_conf` parameter for this single purpose,
        // acknowledging this is not ideal and might need further refinement.
        //
        // Re-evaluating: The `is_thermistor` flag is part of `RawTemperatureAdcReadings`.
        // The `convert_raw_adc_to_temperature_data` function takes `RawTemperatureAdcReadings`.
        // The `Bq769x0::read_temperatures` in `lib.rs` now calls this.
        // The `Bq76920Measurements` in the sub-module now stores `TemperatureData`.
        // The `is_thermistor` flag is lost unless we explicitly pass it along.
        //
        // Simplest path forward for now: `AllMeasurementsUsbPayload::bq76920_is_thermistor`
        // will need to be populated based on information that must be present in `AllMeasurements`.
        // Let's add `is_thermistor` to `crate::data_types::Bq76920Measurements`.
        // This means `bq76920_task.rs` must determine and set this.
        // And `bq769x0_async_rs::Bq76920Measurements` also needs it.
        // This is becoming a cascade.
        //
        // Alternative for `to_usb_payload`: if `ntc_params` were used for conversion (which we'd know if `bq76920_conf.ntc_params.is_some()`),
        // then `is_thermistor` is true. This reintroduces a dependency on `bq76920_conf`.
        //
        // Let's assume `Bq76920RuntimeConfig` is *still passed* to `to_usb_payload` for now,
        // solely for the `is_thermistor` flag, and we'll simplify `shared.rs` later if possible.
        // This means the previous removal of `bq76920_conf` from the signature was premature.
        // I will revert that part of the plan for `to_usb_payload`'s signature for now.

        let bq76920_is_thermistor_flag = self.bq76920.core_measurements.is_thermistor_mode;

        AllMeasurementsUsbPayload {
            bq25730_adc_vbat_mv,
            bq25730_adc_vsys_mv,
            bq25730_adc_ichg_ma,
            bq25730_adc_idchg_ma,
            bq25730_adc_iin_ma,
            bq25730_adc_psys_mv,
            bq25730_adc_vbus_mv,
            bq25730_adc_cmpin_mv,

            bq76920_cell1_mv: self.bq76920.core_measurements.cell_voltages.voltages[0],
            bq76920_cell2_mv: self.bq76920.core_measurements.cell_voltages.voltages[1],
            bq76920_cell3_mv: self.bq76920.core_measurements.cell_voltages.voltages[2],
            bq76920_cell4_mv: self.bq76920.core_measurements.cell_voltages.voltages[3],
            bq76920_cell5_mv: self.bq76920.core_measurements.cell_voltages.voltages[4], // Assuming N=5
            bq76920_ts1_temp_0_01c: ts1_temp_0_01c_val,
            bq76920_ts2_present: self.bq76920.core_measurements.temperatures.ts2.is_some() as u8,
            bq76920_ts2_temp_0_01c: ts2_temp_0_01c_val,
            bq76920_ts3_present: self.bq76920.core_measurements.temperatures.ts3.is_some() as u8,
            bq76920_ts3_temp_0_01c: ts3_temp_0_01c_val,
            bq76920_is_thermistor: bq76920_is_thermistor_flag as u8, // Updated
            bq76920_current_ma: self.bq76920.core_measurements.current_ma, // Updated field name
            bq76920_system_status_mask: self.bq76920.core_measurements.system_status.0.bits(),
            bq76920_mos_status_mask: self.bq76920.core_measurements.mos_status.0.bits(),

            ina226_voltage_f32: self.ina226.voltage,
            ina226_current_f32: self.ina226.current,
            ina226_power_f32: self.ina226.power,

            bq25730_charger_status_flags: self.bq25730_alerts.charger_status.to_u16(),
            bq25730_prochot_status_flags: self.bq25730_alerts.prochot_status.to_u16(),

            bq76920_alerts_system_status_mask: self.bq76920_alerts.system_status.0.bits(),
        }
    }
}

/// Payload structure for USB communication, containing flattened data from AllMeasurements.
#[derive(Debug, Copy, Clone, PartialEq, binrw::BinWrite, defmt::Format)] // Removed binrw::BinRead
pub struct AllMeasurementsUsbPayload {
    // Fields from Bq25730Measurements -> AdcMeasurements
    pub bq25730_adc_vbat_mv: u16,  // Was bq25730_adc_vbat_raw, unit: mV
    pub bq25730_adc_vsys_mv: u16,  // Was bq25730_adc_vsys_raw, unit: mV
    pub bq25730_adc_ichg_ma: u16,  // Was bq25730_adc_ichg_raw, unit: mA
    pub bq25730_adc_idchg_ma: u16, // Was bq25730_adc_idchg_raw, unit: mA
    pub bq25730_adc_iin_ma: u16,   // Was bq25730_adc_iin_raw, unit: mA
    pub bq25730_adc_psys_mv: u16, // Was bq25730_adc_psys_raw, unit: mV (represents power related voltage)
    pub bq25730_adc_vbus_mv: u16, // Was bq25730_adc_vbus_raw, unit: mV
    pub bq25730_adc_cmpin_mv: u16, // Was bq25730_adc_cmpin_raw, unit: mV

    // Fields from Bq76920Measurements -> Bq76920CoreMeasurements<N>
    pub bq76920_cell1_mv: i32,       // Unchanged
    pub bq76920_cell2_mv: i32,       // Unchanged
    pub bq76920_cell3_mv: i32,       // Unchanged
    pub bq76920_cell4_mv: i32,       // Unchanged
    pub bq76920_cell5_mv: i32,       // Unchanged (assuming N=5 for this example)
    pub bq76920_ts1_temp_0_01c: i16, // Was bq76920_ts1_raw_adc, unit: 0.01 °C
    pub bq76920_ts2_present: u8,     // Unchanged
    pub bq76920_ts2_temp_0_01c: i16, // Was bq76920_ts2_raw_adc, unit: 0.01 °C (use i16::MIN if not present)
    pub bq76920_ts3_present: u8,     // Unchanged
    pub bq76920_ts3_temp_0_01c: i16, // Was bq76920_ts3_raw_adc, unit: 0.01 °C (use i16::MIN if not present)
    pub bq76920_is_thermistor: u8,   // Unchanged
    pub bq76920_current_ma: i32,     // Unchanged

    pub bq76920_system_status_mask: u8, // Was bq76920_system_status_bits
    pub bq76920_mos_status_mask: u8,    // Was bq76920_mos_status_bits

    // Fields from Ina226Measurements
    pub ina226_voltage_f32: f32, // Unchanged
    pub ina226_current_f32: f32, // Unchanged
    pub ina226_power_f32: f32,   // Unchanged

    // Fields from Bq25730Alerts
    pub bq25730_charger_status_flags: u16, // Was bq25730_charger_status_raw_u16
    pub bq25730_prochot_status_flags: u16, // Was bq25730_prochot_status_raw_u16

    // Fields from Bq76920Alerts
    pub bq76920_alerts_system_status_mask: u8, // Was bq76920_alerts_system_status_bits
}

// Removed the complex Format impl for AllMeasurements<N>
// It was potentially incorrect regarding NTC parameter handling during logging.
// We can rely on the Format impl for AllMeasurementsUsbPayload if needed,
// or add a simpler Format impl here later.
