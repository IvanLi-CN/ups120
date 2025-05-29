use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq25730_async_rs::Bq25730;

 // Removed unused imports
use crate::shared::{Bq25730AlertsPublisher, Bq25730MeasurementsPublisher}; // Import Bq25730MeasurementsPublisher

#[embassy_executor::task]
pub async fn bq25730_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    bq25730_alerts_publisher: Bq25730AlertsPublisher<'static>,
    bq25730_measurements_publisher: Bq25730MeasurementsPublisher<'static>, // Add publisher for measurements
) {
    info!("BQ25730 task started.");

    // Create temporary I2cDevice instance for BQ25730
    let mut bq25730 = Bq25730::new(i2c_bus, address, 4); // Use a clone for Bq25730

    loop {
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
                // info!("  VAP Fail: {}", status.msb_flags.contains(bq25730_async_rs::registers::ProchotStatusMsbFlags::STAT_VAP_FAIL)); // STAT_VAP_FAIL not found in new version
                // info!("  Exit VAP: {}", status.msb_flags.contains(bq25730_async_rs::registers::ProchotStatusMsbFlags::STAT_EXIT_VAP)); // STAT_EXIT_VAP not found in new version
                // info!("  IDCHG2 Triggered: {}", status.lsb_flags.contains(bq25730_async_rs::registers::ProchotOption1Flags::PP_IDCHG2)); // STAT_IDCHG2 not found in new version
                // info!("  PTM Operation: {}", status.lsb_flags.contains(bq25730_async_rs::registers::ChargeOption3Flags::EN_PTM)); // STAT_PTM not found in new version
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
        Timer::after(Duration::from_secs(1)).await; // Adjust delay as needed
    }
}
