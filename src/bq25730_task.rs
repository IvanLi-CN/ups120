use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq25730_async_rs::Bq25730;
// use bq25730_async_rs::RegisterAccess; // Import the RegisterAccess trait // Commented out as unused
use bq25730_async_rs::registers::{
    ChargeOption0Flags, ChargeOption3MsbFlags, // Register as Bq25730Register, // Commented out as unused
};
// use bq25730_async_rs::data_types::{ChargeOption0, ChargeOption3}; // Import register data types // Commented out as unused
use bq769x0_async_rs::registers::{SysCtrl2Flags as Bq76920SysCtrl2Flags, SysStatFlags as Bq76920SysStatFlags}; // Import BQ76920 flags with alias

use crate::shared::{
    Bq25730AlertsPublisher, Bq25730MeasurementsPublisher, Bq76920MeasurementsSubscriber,
};

#[embassy_executor::task]
pub async fn bq25730_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    bq25730_alerts_publisher: Bq25730AlertsPublisher<'static>,
    bq25730_measurements_publisher: Bq25730MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    info!("BQ25730 task started.");

    let mut bq25730 = Bq25730::new(i2c_bus, address, 4);

    loop {
        // Receive latest BQ76920 measurements
        let bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;
        info!("Received BQ76920 measurements in BQ25730 task.");

        // --- Reading BQ25730 Data ---
        info!("--- Reading BQ25730 Data ---");

        // Read Charger Status
        let bq25730_charger_status = match bq25730.read_charger_status().await {
            Ok(status) => {
                info!("BQ25730 Charger Status:");
                info!(
                    "  Input Present: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::STAT_AC)
                );
                info!(
                    "  ICO Complete: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::ICO_DONE)
                );
                info!(
                    "  In VAP Mode: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_VAP)
                );
                info!(
                    "  In VINDPM: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_VINDPM)
                );
                info!(
                    "  In IIN_DPM: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_IIN_DPM)
                );
                info!(
                    "  In Fast Charge: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_FCHRG)
                );
                info!(
                    "  In Pre-Charge: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_PCHRG)
                );
                info!(
                    "  In OTG Mode: {}",
                    status
                        .status_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFlags::IN_OTG)
                );
                info!(
                    "  Fault ACOV: {}",
                    status
                        .fault_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_ACOV)
                );
                info!(
                    "  Fault BATOC: {}",
                    status.fault_flags.contains(
                        bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_BATOC
                    )
                );
                info!(
                    "  Fault ACOC: {}",
                    status
                        .fault_flags
                        .contains(bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_ACOC)
                );
                info!(
                    "  Fault SYSOVP: {}",
                    status.fault_flags.contains(
                        bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP
                    )
                );
                info!(
                    "  Fault VSYS_UVP: {}",
                    status.fault_flags.contains(
                        bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_VSYS_UVP
                    )
                );
                info!(
                    "  Fault Force Converter Off: {}",
                    status.fault_flags.contains(bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_FORCE_CONVERTER_OFF)
                );
                info!(
                    "  Fault OTG OVP: {}",
                    status.fault_flags.contains(
                        bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_OTG_OVP
                    )
                );
                info!(
                    "  Fault OTG UVP: {}",
                    status.fault_flags.contains(
                        bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_OTG_UVP
                    )
                );
                Some(status)
            }
            Err(e) => {
                error!("Failed to read BQ25730 Charger Status: {:?}", e);
                None
            }
        };

        // Read Prochot Status
        let bq25730_prochot_status = match bq25730.read_prochot_status().await {
            Ok(status) => {
                info!("BQ25730 Prochot Status:");
                info!(
                    "  VINDPM Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_VINDPM)
                );
                info!(
                    "  Comparator Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_COMP)
                );
                info!(
                    "  ICRIT Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_ICRIT)
                );
                info!(
                    "  INOM Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_INOM)
                );
                info!(
                    "  IDCHG1 Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_IDCHG1)
                );
                info!(
                    "  VSYS Triggered: {}",
                    status
                        .lsb_flags
                        .contains(bq25730_async_rs::registers::ProchotStatusFlags::STAT_VSYS)
                );
                info!(
                    "  Battery Removal: {}",
                    status.lsb_flags.contains(
                        bq25730_async_rs::registers::ProchotStatusFlags::STAT_BAT_REMOVAL
                    )
                );
                info!(
                    "  Adapter Removal: {}",
                    status.lsb_flags.contains(
                        bq25730_async_rs::registers::ProchotStatusFlags::STAT_ADPT_REMOVAL
                    )
                );
                Some(status)
            }
            Err(e) => {
                error!("Failed to read BQ25730 Prochot Status: {:?}", e);
                None
            }
        };

        // 更新并发布 BQ25730 告警信息（包含 Prochot Status）
        if let (Some(charger_status), Some(prochot_status)) =
            (bq25730_charger_status, bq25730_prochot_status)
        {
            let alerts = crate::data_types::Bq25730Alerts {
                charger_status,
                prochot_status,
            };
            bq25730_alerts_publisher.publish_immediate(alerts);
        }

        // Read ADC Measurements for BQ25730
        let bq25730_adc_measurements = match bq25730.read_adc_measurements().await {
            Ok(measurements) => {
                info!("BQ25730 ADC Measurements:");
                info!("  PSYS: {} mW", measurements.psys.0);
                info!("  VBUS: {} mV", measurements.vbus.0);
                info!("  IDCHG: {} mA", measurements.idchg.0);
                info!("  ICHG: {} mA", measurements.ichg.0);
                info!("  CMPIN: {} mV", measurements.cmpin.0);
                info!("  IIN: {} mA", measurements.iin.milliamps);
                info!("  VBAT: {} mV", measurements.vbat.0);
                info!("  VSYS: {} mV", measurements.vsys.0);
                Some(measurements)
            }
            Err(e) => {
                error!("Failed to read BQ25730 ADC Measurements: {:?}", e);
                None
            }
        };

        // Implement BQ25730 charge/discharge control based on BQ76920 MOS and System status
        // Directly use mos_status and sys_status as they are not Options here
        let mos_status = bq76920_measurements.core_measurements.mos_status;
        let sys_status = bq76920_measurements.core_measurements.system_status;

        let bq76920_charge_allowed_by_mos = mos_status.0.contains(Bq76920SysCtrl2Flags::CHG_ON);
        let bq76920_discharge_allowed_by_mos = mos_status.0.contains(Bq76920SysCtrl2Flags::DSG_ON);

        // Check BQ76920 system status for faults that should prevent charging
        let can_charge_from_sys_status = !sys_status.0.intersects(
            Bq76920SysStatFlags::OV // Over-voltage
            // Add other critical flags if needed, e.g., OVR_TEMP if it's a charging fault
        );

        // Check BQ76920 system status for faults that should prevent discharging
        let can_discharge_from_sys_status = !sys_status.0.intersects(
            Bq76920SysStatFlags::UV   | // Under-voltage
            Bq76920SysStatFlags::SCD  | // Short-circuit discharge
            Bq76920SysStatFlags::OCD    // Over-current discharge
            // Add other critical flags if needed
        );

        // `final_charge_permission` and `final_discharge_permission` are now declared in the correct scope
        let final_charge_permission = bq76920_charge_allowed_by_mos && can_charge_from_sys_status;
        let final_discharge_permission = bq76920_discharge_allowed_by_mos && can_discharge_from_sys_status;

        info!("BQ76920 MOS: CHG_ON={}, DSG_ON={}", bq76920_charge_allowed_by_mos, bq76920_discharge_allowed_by_mos);
        info!("BQ76920 SYS: CanCharge={}, CanDischarge={}", can_charge_from_sys_status, can_discharge_from_sys_status);
        info!("Final BQ25730 Control: AllowCharge={}, AllowDischarge={}", final_charge_permission, final_discharge_permission);

        // Control BQ25730 charging (via ChargeOption0.CHRG_INHIBIT)
        match bq25730.read_charge_option0().await {
            Ok(mut charge_option_0) => {
                if final_charge_permission {
                    let lsb = &mut charge_option_0.lsb_flags;
                    lsb.remove(ChargeOption0Flags::CHRG_INHIBIT); // CHRG_INHIBIT = 0 to enable charging
                    info!("Attempting to ENABLE BQ25730 charging (CHRG_INHIBIT=0).");
                } else {
                    let lsb = &mut charge_option_0.lsb_flags;
                    lsb.insert(ChargeOption0Flags::CHRG_INHIBIT); // CHRG_INHIBIT = 1 to disable charging
                    info!("Attempting to DISABLE BQ25730 charging (CHRG_INHIBIT=1).");
                }
                match bq25730.set_charge_option0(charge_option_0).await {
                    Ok(_) => info!("BQ25730 ChargeOption0 (CHRG_INHIBIT) updated."),
                    Err(e) => error!("Failed to update BQ25730 ChargeOption0 (CHRG_INHIBIT): {:?}", e),
                }
            },
            Err(e) => error!("Failed to read BQ25730 ChargeOption0 for charging control: {:?}", e),
        }

        // Control BQ25730 discharging (OTG Mode via ChargeOption3.EN_OTG)
        match bq25730.read_charge_option3().await {
            Ok(mut charge_option_3) => {
                if final_discharge_permission {
                    let msb = &mut charge_option_3.msb_flags;
                    msb.insert(ChargeOption3MsbFlags::EN_OTG); // EN_OTG = 1 to enable OTG
                    info!("Attempting to ENABLE BQ25730 discharging (EN_OTG=1).");
                } else {
                    let msb = &mut charge_option_3.msb_flags;
                    msb.remove(ChargeOption3MsbFlags::EN_OTG); // EN_OTG = 0 to disable OTG
                    info!("Attempting to DISABLE BQ25730 discharging (EN_OTG=0).");
                }
                match bq25730.set_charge_option3(charge_option_3).await {
                    Ok(_) => info!("BQ25730 ChargeOption3 (EN_OTG) updated."),
                    Err(e) => error!("Failed to update BQ25730 ChargeOption3 (EN_OTG): {:?}", e),
                }
            },
            Err(e) => error!("Failed to read BQ25730 ChargeOption3 for OTG control: {:?}", e),
        }

        // Implement battery charge logic based on BQ76920 total battery voltage and final_charge_permission
        let total_voltage_mv: u16 = bq76920_measurements.core_measurements.cell_voltages.voltages.iter().sum();
        info!("BQ76920 Total Voltage: {} mV for BQ25730 parameter decision.", total_voltage_mv);

        // Define charging parameters
        let charge_stop_threshold_mv = 3600 * 5; // Example: Stop charging if total voltage is above 18V (3.6V per cell for 5 cells)
        let charge_current_ma = 1000;            // Example: Charge current 1000mA
        let charge_voltage_mv = 18000;           // Example: Charge voltage 18000mV (18V)

        // Set charging parameters only if charging is permitted AND battery voltage is below stop threshold.
        // CHRG_INHIBIT bit itself is controlled by `final_charge_permission` logic block above.
        if final_charge_permission {
            if total_voltage_mv < charge_stop_threshold_mv {
                info!("Setting/Re-asserting BQ25730 charge voltage ({} mV) and current ({} mA).", charge_voltage_mv, charge_current_ma);
                match bq25730.set_charge_voltage(bq25730_async_rs::data_types::ChargeVoltage(charge_voltage_mv)).await {
                    Ok(_) => info!("BQ25730 charge voltage set to {} mV.", charge_voltage_mv),
                    Err(e) => error!("Failed to set BQ25730 charge voltage: {:?}", e),
                }
                match bq25730.set_charge_current(bq25730_async_rs::data_types::ChargeCurrent(charge_current_ma)).await {
                    Ok(_) => info!("BQ25730 charge current set to {} mA.", charge_current_ma),
                    Err(e) => error!("Failed to set BQ25730 charge current: {:?}", e),
                }
            } else { // total_voltage_mv >= charge_stop_threshold_mv
                info!("Total voltage ({}) at or above stop threshold ({}). Charging should be inhibited by CHRG_INHIBIT logic or BQ25730 internal protection. Not setting parameters.",
                      total_voltage_mv, charge_stop_threshold_mv);
            }
        } else { // !final_charge_permission
            info!("BQ25730 charging is not permitted by BQ76920 (final_charge_permission=false). Not setting charge parameters.");
        }

        // Construct BQ25730 measurements
        let bq25730_measurements = crate::data_types::Bq25730Measurements {
            adc_measurements: bq25730_adc_measurements.unwrap_or_else(|| {
                bq25730_async_rs::data_types::AdcMeasurements {
                    psys: bq25730_async_rs::data_types::AdcPsys::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.psys.0 / 12) as u8), // Convert back to raw for default
                    ),
                    vbus: bq25730_async_rs::data_types::AdcVbus::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.vbus.0 / 96) as u8), // Convert back to raw for default
                    ),
                    idchg: bq25730_async_rs::data_types::AdcIdchg::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.idchg.0 / 512) as u8), // Convert back to raw for default
                    ),
                    ichg: bq25730_async_rs::data_types::AdcIchg::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.ichg.0 / 128) as u8), // Convert back to raw for default
                    ),
                    cmpin: bq25730_async_rs::data_types::AdcCmpin::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.cmpin.0 / 12) as u8), // Convert back to raw for default
                    ),
                    iin: bq25730_async_rs::data_types::AdcIin::from_u8(
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.iin.milliamps / 100) as u8), // Convert back to raw for default
                        true, // Assuming 5mOhm sense resistor for default
                    ),
                    vbat: bq25730_async_rs::data_types::AdcVbat::from_register_value(
                        0, // _lsb: u8
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.vbat.0 / 64) as u8), // msb: u8
                        bq25730_async_rs::data_types::AdcVbat::OFFSET_MV, // offset_mv: u16
                    ),
                    vsys: bq25730_async_rs::data_types::AdcVsys::from_register_value(
                        0, // _lsb: u8
                        bq25730_adc_measurements
                            .as_ref()
                            .map_or(0, |m| (m.vsys.0 / 64) as u8), // msb: u8
                        bq25730_async_rs::data_types::AdcVsys::OFFSET_MV, // offset_mv: u16
                    ),
                }
            }),
        };
        // Publish BQ25730 measurements
        bq25730_measurements_publisher.publish_immediate(bq25730_measurements);
        // Add other BQ25730 measurement fields here when implemented
        info!("BQ25730 task loop end."); // Add a log to mark the end of the loop iteration
        Timer::after(Duration::from_secs(1)).await; // Adjust delay as needed
    }
}
