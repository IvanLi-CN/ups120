// use defmt::Format; // Removed unused import

use bq769x0_async_rs::data_types::{Bq76920Measurements as Bq76920CoreMeasurements, SystemStatus};
use sc8815::{AdcMeasurements as Sc8815AdcMeasurements, SC8815Status};

// use crate::shared::Bq76920RuntimeConfig; // Removed as Bq76920RuntimeConfig is no longer needed by to_usb_payload

// Removed BQ25730 related structures as we're using SC8815 now

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
/// INA226 测量数据
#[allow(dead_code)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct Ina226Measurements {
    pub voltage: f32,
    pub current: f32,
    pub power: f32, // 假设需要功率，如果不需要可以调整
}

/// SC8815 测量数据
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct Sc8815Measurements {
    pub adc_measurements: Sc8815AdcMeasurements,
}

/// SC8815 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct Sc8815Alerts {
    pub device_status: SC8815Status,
    /// Firmware currently requests the charger to be active (CE low, PSTOP low).
    pub expected_charging: bool,
    /// Filtered SC8815 telemetry confirms measurable charge current.
    pub charging_confirmed: bool,
    /// OV pause (charger power stage gated via PSTOP for cooldown), not necessarily a live OV fault bit.
    pub ov_pause_active: bool,
    /// Severe imbalance pause (Δcell >= threshold); charger power stage gated until Δ falls below release threshold.
    pub imbalance_pause_active: bool,
}

/// Balancing → Charger coupling signal
///
/// When `require_cv` is true, the charger task shall maintain CV charging
/// (keep session active and avoid termination) until the balancer indicates
/// it is no longer required.
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct BalancingCvRequest {
    /// Request charger to maintain CV (used by charger task)
    pub require_cv: bool,
    /// Whether LED should display balancing overlay (true only when HW balancing active)
    pub overlay: bool,
    /// Severe imbalance indicator (Δcell >= 100 mV)
    pub severe_imbalance: bool,
    /// Request SC8815 to pause charging due to temperature (host-level request)
    pub temp_pause: bool,
}

impl Default for BalancingCvRequest {
    fn default() -> Self {
        Self {
            require_cv: false,
            overlay: false,
            severe_imbalance: false,
            temp_pause: false,
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
    #[allow(dead_code)]
    pub fn as_usb_payload(&self) -> AllMeasurementsUsbPayload {
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
            bq76920_total_voltage_mv: self.bq76920.core_measurements.total_voltage_mv, // Corrected access
            bq76920_ts1_temp_0_01c: ts1_temp_0_01c_val,
            bq76920_ts2_present: self.bq76920.core_measurements.temperatures.ts2.is_some() as u8,
            bq76920_ts2_temp_0_01c: ts2_temp_0_01c_val,
            bq76920_ts3_present: self.bq76920.core_measurements.temperatures.ts3.is_some() as u8,
            bq76920_ts3_temp_0_01c: ts3_temp_0_01c_val,
            bq76920_is_thermistor: bq76920_is_thermistor_flag as u8, // Updated
            bq76920_current_ma: self.bq76920.core_measurements.current_ma, // Updated field name
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
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
pub struct AllMeasurementsUsbPayload {
    // Fields from SC8815 ADC measurements
    pub sc8815_adc_vbus_mv: u16, // VBUS voltage in mV
    pub sc8815_adc_vbat_mv: u16, // VBAT voltage in mV
    pub sc8815_adc_ibus_ma: u16, // IBUS current in mA
    pub sc8815_adc_ibat_ma: u16, // IBAT current in mA
    pub sc8815_adc_adin_mv: u16, // ADIN voltage in mV

    // Fields from Bq76920Measurements -> Bq76920CoreMeasurements<N>
    pub bq76920_cell1_mv: i32,         // Unchanged
    pub bq76920_cell2_mv: i32,         // Unchanged
    pub bq76920_cell3_mv: i32,         // Unchanged
    pub bq76920_cell4_mv: i32,         // Unchanged
    pub bq76920_cell5_mv: i32,         // Unchanged (assuming N=5 for this example)
    pub bq76920_total_voltage_mv: i32, // Added: Total voltage of the BQ76920 pack
    pub bq76920_ts1_temp_0_01c: i16,   // Was bq76920_ts1_raw_adc, unit: 0.01 °C
    pub bq76920_ts2_present: u8,       // Unchanged
    pub bq76920_ts2_temp_0_01c: i16, // Was bq76920_ts2_raw_adc, unit: 0.01 °C (use i16::MIN if not present)
    pub bq76920_ts3_present: u8,     // Unchanged
    pub bq76920_ts3_temp_0_01c: i16, // Was bq76920_ts3_raw_adc, unit: 0.01 °C (use i16::MIN if not present)
    pub bq76920_is_thermistor: u8,   // Unchanged
    pub bq76920_current_ma: i32,     // Unchanged

    pub bq76920_system_status_mask: u8, // Was bq76920_system_status_bits
    pub bq76920_mos_status_mask: u8,    // Was bq76920_mos_status_bits

    // Fields from SC8815 device status
    pub sc8815_device_status_flags: u8, // SC8815 device status flags

    // Fields from Bq76920Alerts
    pub bq76920_alerts_system_status_mask: u8, // Was bq76920_alerts_system_status_bits
}

// Removed the complex Format impl for AllMeasurements<N>
// It was potentially incorrect regarding NTC parameter handling during logging.
// We can rely on the Format impl for AllMeasurementsUsbPayload if needed,
// or add a simpler Format impl here later.
