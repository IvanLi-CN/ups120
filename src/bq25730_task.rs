use bq25730_async_rs::data_types::{
    AdcCmpin, AdcIchg, AdcIdchg, AdcIin, AdcMeasurements, AdcPsys, AdcVbat, AdcVbus, AdcVsys,
    ChargeCurrentSetting, ChargeVoltageSetting, OtgCurrentSetting, OtgVoltageSetting,
    VsysMinSetting,
}; // Added AdcVsys
use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq769x0_async_rs::registers::{
    SysCtrl2Flags as Bq76920SysCtrl2Flags, SysStatFlags as Bq76920SysStatFlags,
};
use bq25730_async_rs::RegisterAccess;
use bq25730_async_rs::registers::{
    ChargeOption0Flags, ChargeOption0MsbFlags, ChargeOption1Flags, ChargeOption1MsbFlags,
    ChargeOption3MsbFlags, WatchdogTimerAdjust,
}; // Removed ChargeOption2Flags
use bq25730_async_rs::{Bq25730, SenseResistorValue};

use crate::shared::{
    Bq25730AlertsPublisher, Bq25730MeasurementsPublisher, Bq76920MeasurementsSubscriber,
};
// Default charging parameters
const DEFAULT_CHARGE_CURRENT_MA: u16 = 512;
const DEFAULT_CHARGE_VOLTAGE_MV: u16 = 18000;

/// Embassy task for managing the BQ25730 charger IC.
#[embassy_executor::task]
pub async fn bq25730_task(
    i2c_bus: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    address: u8,
    bq25730_alerts_publisher: Bq25730AlertsPublisher<'static>,
    bq25730_measurements_publisher: Bq25730MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    info!("BQ25730 task started.");

    // Initialize with a Config struct
    let mut config = bq25730_async_rs::data_types::Config::new(
        4,
        SenseResistorValue::R5mOhm,
        SenseResistorValue::R10mOhm,
    );
    config
        .charge_option0
        .msb_flags
        .remove(ChargeOption0MsbFlags::EN_LWPWR);
    config
        .charge_option0
        .msb_flags
        .insert(ChargeOption0MsbFlags::EN_OOA);
    // Set WDTMR_ADJ to 01b (Enabled, 5 sec timeout, suspends charger by setting ChargeCurrent to 0mA on timeout)

    config
        .charge_option0
        .msb_flags
        .set_watchdog_timer(WatchdogTimerAdjust::Sec5);
    config
        .charge_option3
        .msb_flags
        .insert(ChargeOption3MsbFlags::EN_OTG | ChargeOption3MsbFlags::EN_ICO_MODE);
    // config
    //     .charge_option3
    //     .lsb_flags
    //     .insert(ChargeOption3Flags::OTG_VAP_MODE);

    config
        .charge_option1
        .msb_flags
        .insert(ChargeOption1MsbFlags::EN_IBAT);
    config
        .charge_option1
        .lsb_flags
        .insert(ChargeOption1Flags::CMP_REF);

    // Set OtgVoltage to 12V (12000mV) in config
    config.otg_voltage = OtgVoltageSetting::from_millivolts(19000);

    // Set OtgCurrent to 5A (5000mA) in config
    // The conversion requires the battery sense resistor value (rsns_bat)
    let rsns_bat = config.rsns_bat; // Use rsns_bat from the config being built
    config.otg_current = OtgCurrentSetting::from_milliamps(5000, rsns_bat);

    config.vmin_active_protection.set_en_frs(true);
    config.vmin_active_protection.set_vbus_vap_th_mv(9000);
    config.vmin_active_protection.set_vsys_th2_mv(13000);

    let mut bq25730 = Bq25730::new(i2c_bus, address, config);

    // init() will determine the correct rsns from the chip and update bq25730.rsns
    if let Err(e) = bq25730.init().await {
        error!("Failed to initialize BQ25730: {:?}", e);
        // Handle initialization error, perhaps by retrying or stopping the task
        return;
    }

    let initial_charge_current = ChargeCurrentSetting {
        milliamps: 0,
        rsns_bat: bq25730.config().rsns_bat,
    };
    if let Err(e) = bq25730
        .set_charge_current_setting(initial_charge_current)
        .await
    {
        error!(
            "Failed to set initial BQ25730 charge current to 0mA: {:?}",
            e
        );
    }

    let target_charge_voltage = ChargeVoltageSetting::from_millivolts(DEFAULT_CHARGE_VOLTAGE_MV);
    if let Err(e) = bq25730
        .set_charge_voltage_setting(target_charge_voltage)
        .await
    {
        error!("Failed to set BQ25730 target charge voltage: {:?}", e);
    }

    info!("Configuring and enabling BQ25730 ADC for continuous conversion...");
    let adc_option = bq25730_async_rs::data_types::AdcOption {
        msb_flags: bq25730_async_rs::registers::AdcOptionMsbFlags::ADC_CONV
            | bq25730_async_rs::registers::AdcOptionMsbFlags::ADC_START
            | bq25730_async_rs::registers::AdcOptionMsbFlags::ADC_FULLSCALE,
        lsb_flags: bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_CMPIN
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_VBUS
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_PSYS
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_IIN
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_IDCHG
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_ICHG
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_VSYS
            | bq25730_async_rs::registers::AdcOptionFlags::EN_ADC_VBAT,
    };
    if let Err(e) = bq25730.set_adc_option(adc_option).await {
        error!("Failed to set BQ25730 ADC options: {:?}", e);
    }

    match bq25730
        .set_vsys_min_setting(VsysMinSetting::from_millivolts(12000))
        .await
    {
        Ok(()) => { /* Log removed */ }
        Err(e) => error!("Failed to set BQ25730 VsysMin: {}", e),
    }

    loop {
        let bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;

        let bq25730_adc_measurements_option = match bq25730.read_adc_measurements().await {
            Ok(measurements) => {
                info!(
                    "[BQ25730 ADC] VBUS:{}mV, VSYS:{}mV, VBAT:{}mV, ICHG:{}mA, IIN:{}mA, PSYS:{}mV, CMPIN:{}mV, IDCHG:{}mA",
                    measurements.vbus.0,
                    measurements.vsys.0,
                    measurements.vbat.0,
                    measurements.ichg.milliamps,  // Access .milliamps
                    measurements.iin.milliamps,   // Access .milliamps
                    measurements.psys.0,          // Already in mV
                    measurements.cmpin.0,         // Already in mV
                    measurements.idchg.milliamps  // Access .milliamps
                );
                Some(measurements)
            }
            Err(e) => {
                error!("[BQ25730] Failed to read ADC Measurements: {:?}", e);
                None
            }
        };

        let bq25730_charger_status_option = match bq25730.read_charger_status().await {
            Ok(status) => {
                info!(
                    "BQ25730 ChargerStatus: Status={:?}, Fault={:?}",
                    status.status_flags, status.fault_flags
                );
                Some(status)
            }
            Err(e) => {
                error!("Failed to read BQ25730 Charger Status: {:?}", e);
                None
            }
        };
        let bq25730_charger_status = bq25730_charger_status_option;

        if let Some(status) = &bq25730_charger_status {
            if status
                .fault_flags
                .contains(bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP)
            {
                info!("[BQ25730] FAULT_SYSOVP is active.");
                let mut attempt_clear_sys_ovp = false;
                if let Some(adc_measurements) = &bq25730_adc_measurements_option {
                    if adc_measurements.vsys.0 <= 19500 {
                        info!(
                            "[BQ25730] VSYS ({}mV) is not > 19.5V. Conditions met to attempt FAULT_SYSOVP clear.",
                            adc_measurements.vsys.0
                        );
                        attempt_clear_sys_ovp = true;
                    } else {
                        info!(
                            "[BQ25730] VSYS ({}mV) is > 19.5V. Not attempting to clear FAULT_SYSOVP.",
                            adc_measurements.vsys.0
                        );
                    }
                } else {
                    info!(
                        "[BQ25730] ADC measurements not available. Cannot verify VSYS voltage. Not clearing FAULT_SYSOVP."
                    );
                }

                if attempt_clear_sys_ovp {
                    match bq25730
                        .read_register(bq25730_async_rs::registers::Register::ChargerStatus) // Reads LSB by default
                        .await
                    {
                        Ok(mut fault_val) => {
                            let original_fault_val = fault_val;
                            if (original_fault_val & bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP.bits()) != 0 {
                                fault_val &= !bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP.bits();
                                if let Err(e) = bq25730.write_register(bq25730_async_rs::registers::Register::ChargerStatus, fault_val).await {
                                    error!("[BQ25730] Failed to write ChargerStatus LSB to clear FAULT_SYSOVP: {:?}", e);
                                } else {
                                    info!("[BQ25730] Wrote to ChargerStatus LSB (0x{:02x}) to clear FAULT_SYSOVP. Original: 0x{:02x}", fault_val, original_fault_val);
                                }
                            } else {
                                info!("[BQ25730] FAULT_SYSOVP was not set in the re-read of ChargerStatus LSB (before attempted clear). No clear needed or already cleared by previous read.");
                            }
                        }
                        Err(e) => {
                            error!(
                                "[BQ25730] Failed to read ChargerStatus LSB before attempting to clear FAULT_SYSOVP: {:?}",
                                e
                            );
                        }
                    }
                }
            }
        }

        let bq25730_prochot_status = match bq25730.read_prochot_status().await {
            Ok(status) => {
                info!(
                    "BQ25730 ProchotStatus: LSB={:?}, MSB={:?}",
                    status.lsb_flags, status.msb_flags
                );
                Some(status)
            }
            Err(e) => {
                error!("Failed to read BQ25730 Prochot Status: {:?}", e);
                None
            }
        };

        match bq25730.read_iin_host_setting().await {
            Ok(current_iin_host_val) => {
                if let Err(e) = bq25730.set_iin_host_setting(current_iin_host_val).await {
                    error!("[BQ25730] Failed to re-write IIN_HOST for ICO: {:?}", e);
                }
            }
            Err(e) => {
                error!("[BQ25730] Failed to read IIN_HOST before re-write: {:?}", e);
            }
        }

        if let (Some(cs), Some(ps)) = (bq25730_charger_status, bq25730_prochot_status) {
            let alerts = crate::data_types::Bq25730Alerts {
                charger_status: cs,
                prochot_status: ps,
            };
            bq25730_alerts_publisher.publish_immediate(alerts);
        }

        let bq76920_mos_status = bq76920_measurements.core_measurements.mos_status;
        let bq76920_sys_status = bq76920_measurements.core_measurements.system_status;

        let bq76920_charge_fet_enabled =
            bq76920_mos_status.0.contains(Bq76920SysCtrl2Flags::CHG_ON);
        let _bq76920_discharge_fet_enabled =
            bq76920_mos_status.0.contains(Bq76920SysCtrl2Flags::DSG_ON);

        let bq76920_safe_to_charge = !bq76920_sys_status.0.intersects(Bq76920SysStatFlags::OV);

        let _bq76920_safe_to_discharge = !bq76920_sys_status.0.intersects(
            Bq76920SysStatFlags::UV | Bq76920SysStatFlags::SCD | Bq76920SysStatFlags::OCD,
        );

        let final_charge_permission = bq76920_charge_fet_enabled && bq76920_safe_to_charge;

        // Log key register values for ICHG debugging
        match bq25730.read_charge_current_setting().await {
            Ok(cc) => info!(
                "[BQ25730 DEBUG] ChargeCurrent: {} mA (Raw: {})",
                cc.milliamps,
                cc.to_raw()
            ),
            Err(e) => error!("[BQ25730 DEBUG] Failed to read ChargeCurrent: {:?}", e),
        }
        match bq25730.read_charge_option0().await {
            Ok(co0) => info!(
                "[BQ25730 DEBUG] ChargeOption0: LSB=0x{:02x}, MSB=0x{:02x}",
                co0.lsb_flags.bits(),
                co0.msb_flags.bits()
            ),
            Err(e) => error!("[BQ25730 DEBUG] Failed to read ChargeOption0: {:?}", e),
        }
        match bq25730.read_iin_host_setting().await {
            Ok(iin_host) => info!(
                "[BQ25730 DEBUG] IIN_HOST: {} mA (Raw: {})",
                iin_host.milliamps,
                iin_host.to_raw(bq25730.config().rsns_ac) // Pass rsns_ac for to_raw
            ),
            Err(e) => error!("[BQ25730 DEBUG] Failed to read IIN_HOST: {:?}", e),
        }
        info!(
            "[BQ25730 DEBUG] final_charge_permission: {}",
            final_charge_permission
        );
        match bq25730.read_charge_option0().await {
            Ok(mut charge_option_0) => {
                let original_lsb_flags_val = charge_option_0.lsb_flags.bits();
                let original_lsb_as_flags =
                    ChargeOption0Flags::from_bits_truncate(original_lsb_flags_val);

                charge_option_0
                    .msb_flags
                    .remove(ChargeOption0MsbFlags::EN_LWPWR);
                charge_option_0
                    .lsb_flags
                    .insert(ChargeOption0Flags::IADPT_GAIN);

                if final_charge_permission {
                    let chrg_inhibit_was_set =
                        original_lsb_as_flags.contains(ChargeOption0Flags::CHRG_INHIBIT);
                    charge_option_0
                        .lsb_flags
                        .remove(ChargeOption0Flags::CHRG_INHIBIT);
                    if chrg_inhibit_was_set {
                        info!(
                            "[BQ25730] Charging permitted by BQ76920. Clearing CHRG_INHIBIT (was set)."
                        );
                    }
                } else {
                    let chrg_inhibit_was_set =
                        original_lsb_as_flags.contains(ChargeOption0Flags::CHRG_INHIBIT);
                    charge_option_0
                        .lsb_flags
                        .insert(ChargeOption0Flags::CHRG_INHIBIT);
                    if chrg_inhibit_was_set {
                        info!(
                            "[BQ25730] Charging inhibited by BQ76920. CHRG_INHIBIT was already set."
                        );
                    } else {
                        info!(
                            "[BQ25730] Charging inhibited by BQ76920. Setting CHRG_INHIBIT (was clear)."
                        );
                    }
                }

                if let Err(e) = bq25730.set_charge_option0(charge_option_0).await {
                    error!(
                        "[BQ25730] Failed to write ChargeOption0 to control CHRG_INHIBIT: {:?}",
                        e
                    );
                }
            }
            Err(e) => error!(
                "[BQ25730] Failed to read ChargeOption0 for control: {:?}",
                e
            ),
        }

        if final_charge_permission {
            if let Err(e) = bq25730
                .set_charge_voltage_setting(ChargeVoltageSetting::from_millivolts(
                    DEFAULT_CHARGE_VOLTAGE_MV,
                ))
                .await
            {
                error!("[BQ25730] Failed to set charge voltage: {:?}", e);
            }
            if let Err(e) = bq25730
                .set_charge_current_setting(ChargeCurrentSetting {
                    milliamps: DEFAULT_CHARGE_CURRENT_MA,
                    rsns_bat: bq25730.config().rsns_bat,
                })
                .await
            {
                error!("[BQ25730] Failed to set charge current: {:?}", e);
            }
        } else {
            warn!("[BQ25730] Skipping charge voltage and current settings.");
        }

        let bq25730_measurements_payload = crate::data_types::Bq25730Measurements {
            adc_measurements: bq25730_adc_measurements_option.unwrap_or_else(|| {
                // Directly use the specific rsns_bat or rsns_ac from bq25730 instance
                AdcMeasurements {
                    // Use AdcMeasurements directly as it's in scope
                    vbat: AdcVbat(0),
                    vsys: AdcVsys(0),
                    ichg: AdcIchg {
                        milliamps: 0,
                        rsns_bat: bq25730.config().rsns_bat, // Use rsns_bat from config
                    },
                    idchg: AdcIdchg {
                        milliamps: 0,
                        rsns_bat: bq25730.config().rsns_bat, // Use rsns_bat from config
                    },
                    iin: AdcIin {
                        milliamps: 0,
                        rsns_ac: bq25730.config().rsns_ac, // Use rsns_ac from config
                    },
                    psys: AdcPsys(0),
                    vbus: AdcVbus(0),
                    cmpin: AdcCmpin(0),
                }
            }),
        };
        bq25730_measurements_publisher.publish_immediate(bq25730_measurements_payload);

        Timer::after(Duration::from_secs(1)).await;
    }
}
