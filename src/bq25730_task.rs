use bq25730_async_rs::data_types::ChargeCurrent;
// use bq25730_async_rs::data_types::ChargeOption3; // Unused import
use bq25730_async_rs::data_types::ChargeVoltage;
use bq25730_async_rs::data_types::VsysMin;
// use bq25730_async_rs::registers::ChargeOption3Flags; // Unused import
use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use bq769x0_async_rs::registers::{
    SysCtrl2Flags as Bq76920SysCtrl2Flags, SysStatFlags as Bq76920SysStatFlags,
};
use bq25730_async_rs::Bq25730;
use bq25730_async_rs::RegisterAccess; // Import the RegisterAccess trait
use bq25730_async_rs::registers::{
    ChargeOption0Flags,
    ChargeOption0MsbFlags,
    // ChargeOption3MsbFlags, // Unused import
    // Register,              // Unused import
};

use crate::shared::{
    Bq25730AlertsPublisher, Bq25730MeasurementsPublisher, Bq76920MeasurementsSubscriber,
};

// Default charging parameters
const DEFAULT_CHARGE_CURRENT_MA: u16 = 256;
const DEFAULT_CHARGE_VOLTAGE_MV: u16 = 18000;

// use bq25730_async_rs::registers::ChargerStatusFaultFlags; // Unused import

/// Embassy task for managing the BQ25730 charger IC.
///
/// This task is responsible for:
/// 1. Initializing the BQ25730 chip:
///    - Setting sense resistor configurations (`ChargeOption1`).
///    - Initially setting charge current to 0 mA (inhibiting charging).
///    - Setting a target charge voltage.
///    - Configuring and enabling the ADC for continuous conversion of various parameters
///      (VBUS, VSYS, VBAT, ICHG, IDCHG, IIN, CMPIN, PSYS).
///    - Setting a minimum system voltage (`VsysMin`).
/// 2. In a continuous loop:
///    - Subscribing to and receiving the latest measurement data from the `bq76920_task`.
///      This data includes BQ76920's MOS FET status and system fault status, which are
///      critical for deciding BQ25730's charging behavior.
///    - Reading BQ25730's internal status:
///      - `ChargerStatus` (including fault flags).
///      - `ProchotStatus`.
///    - Attempting to re-trigger Input Current Optimizer (ICO) by re-writing `IIN_HOST`.
///    - Publishing BQ25730's alert information (charger status, prochot status).
///    - Reading BQ25730's ADC measurements.
///    - **Core Charging Control Logic**:
///      - Determining `final_charge_permission` based on:
///        - BQ76920's Charge FET status (`CHG_ON` bit in `SYS_CTRL2`).
///        - BQ76920's system fault status (e.g., Overvoltage `OV`).
///      - Controlling the BQ25730's `CHRG_INHIBIT` bit (in `ChargeOption0` register) based
///        on `final_charge_permission`. If permission is granted, `CHRG_INHIBIT` is cleared;
///        otherwise, it's set to inhibit charging.
///      - (Potentially conditionally) setting the BQ25730's charge voltage and charge current parameters.
///        The current implementation appears to set these parameters in each loop iteration
///        regardless of `final_charge_permission` or battery voltage, which might be intended
///        as a re-assertion of parameters or might need further refinement based on charging strategy.
///    - Publishing BQ25730's ADC measurement data.
///
/// # Arguments
///
/// * `i2c_bus`: A shared I2C bus device for communication with the BQ25730.
/// * `address`: The I2C address of the BQ25730 chip.
/// * `bq25730_alerts_publisher`: Publisher for sending BQ25730 alert data.
/// * `bq25730_measurements_publisher`: Publisher for sending BQ25730 measurement data.
/// * `bq76920_measurements_subscriber`: Subscriber for receiving measurement data from the BQ76920 task.
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

    let charge_option1 = bq25730_async_rs::data_types::ChargeOption1 {
        msb_flags: bq25730_async_rs::registers::ChargeOption1MsbFlags::from_bits_truncate(0x37),
        lsb_flags: bq25730_async_rs::registers::ChargeOption1Flags::from_bits_truncate(0x01),
    };
    if let Err(e) = bq25730.set_charge_option1(charge_option1).await {
        error!(
            "Failed to set BQ25730 ChargeOption1 (sense resistors): {:?}",
            e
        );
    } 

    let initial_charge_current = ChargeCurrent(0);
    if let Err(e) = bq25730.set_charge_current(initial_charge_current).await {
        error!(
            "Failed to set initial BQ25730 charge current to 0mA: {:?}",
            e
        );
    } 

    let target_charge_voltage = ChargeVoltage(18000);
    if let Err(e) = bq25730.set_charge_voltage(target_charge_voltage).await {
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

    match bq25730.set_vsys_min(VsysMin(12000)).await {
        Ok(()) => { /* Log removed */ }
        Err(e) => error!("Failed to set BQ25730 VsysMin: {}", e),
    }

    loop {
        let bq76920_measurements = bq76920_measurements_subscriber.next_message_pure().await;

        // Read ADC measurements first to have fresh VSYS data for fault handling
        let bq25730_adc_measurements_option = match bq25730.read_adc_measurements().await {
            Ok(measurements) => {
                info!(
                    "[BQ25730 ADC] VBUS:{}mV, VSYS:{}mV, VBAT:{}mV, ICHG:{}mA, IIN:{}mA, PSYS:{}raw, CMPIN:{}raw, IDCHG:{}raw",
                    measurements.vbus.0,
                    measurements.vsys.0,
                    measurements.vbat.0,
                    measurements.ichg.0,
                    measurements.iin.milliamps,
                    measurements.psys.0,
                    measurements.cmpin.0,
                    measurements.idchg.0
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

        // Attempt to clear FAULT_SYSOVP if it's set
        if let Some(status) = &bq25730_charger_status {
            // Borrow charger_status
            if status
                .fault_flags
                .contains(bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP)
            {
                info!("[BQ25730] FAULT_SYSOVP is active.");

                let mut attempt_clear_sys_ovp = false;
                if let Some(adc_measurements) = &bq25730_adc_measurements_option {
                    // Use the ADC measurements read at the start of the loop
                    if adc_measurements.vsys.0 <= 19500 {
                        // VSYS is in mV
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
                        .read_register(bq25730_async_rs::registers::Register::ChargerStatus)
                        .await
                    {
                        Ok(mut fault_msb) => {
                            let original_fault_msb = fault_msb;
                            // Check if FAULT_SYSOVP is actually set in this fresh read before clearing
                            if (original_fault_msb & bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP.bits()) != 0 {
                                fault_msb &= !bq25730_async_rs::registers::ChargerStatusFaultFlags::FAULT_SYSOVP.bits(); // Clear bit 4
                                if let Err(e) = bq25730.write_register(bq25730_async_rs::registers::Register::ChargerStatus, fault_msb).await {
                                    error!("[BQ25730] Failed to write ChargerStatusMsb to clear FAULT_SYSOVP: {:?}", e);
                                } else {
                                    info!("[BQ25730] Wrote to ChargerStatusMsb (0x{:02x}) to clear FAULT_SYSOVP. Original: 0x{:02x}", fault_msb, original_fault_msb);
                                    // Status will be re-read at the beginning of the next loop iteration, immediate confirmation removed.
                                }
                            } else {
                                info!("[BQ25730] FAULT_SYSOVP was not set in the re-read of ChargerStatusMsb (before attempted clear). No clear needed or already cleared by previous read.");
                            }
                        }
                        Err(e) => {
                            error!(
                                "[BQ25730] Failed to read ChargerStatusMsb before attempting to clear FAULT_SYSOVP: {:?}",
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
                    "BQ25730 ProchotStatus: LSB=0x{:02x}, MSB=0x{:02x}",
                    status.lsb_flags.bits(),
                    status.msb_flags.bits()
                );
                Some(status)
            }
            Err(e) => {
                error!("Failed to read BQ25730 Prochot Status: {:?}", e);
                None
            }
        };

        match bq25730.read_iin_host().await {
            Ok(current_iin_host_val) => {
                if let Err(e) = bq25730.set_iin_host(current_iin_host_val).await {
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

        // ADC measurements are now read at the beginning of the loop.
        // The variable `bq25730_adc_measurements_option` holds the Option<AdcMeasurements>.

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

        match bq25730.read_charge_option0().await {
            Ok(mut charge_option_0) => {
                let original_lsb_flags_val = charge_option_0.lsb_flags.bits(); // Get u8 value
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
                    } // else {
                //    info!("[BQ25730] Charging permitted by BQ76920. CHRG_INHIBIT was already clear.");
                // }
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

        // let charge_current_ma = 256; // Moved to const
        // let charge_voltage_mv = 18000; // Moved to const

        if final_charge_permission {
            if let Err(e) = bq25730
                .set_charge_voltage(ChargeVoltage(DEFAULT_CHARGE_VOLTAGE_MV))
                .await
            {
                error!("[BQ25730] Failed to set charge voltage: {:?}", e);
            }
            if let Err(e) = bq25730
                .set_charge_current(ChargeCurrent(DEFAULT_CHARGE_CURRENT_MA))
                .await
            {
                error!("[BQ25730] Failed to set charge current: {:?}", e);
            }
        }

        let bq25730_measurements_payload = crate::data_types::Bq25730Measurements {
            adc_measurements: bq25730_adc_measurements_option.unwrap_or_else(|| {
                // Use the option variable here
                bq25730_async_rs::data_types::AdcMeasurements {
                    vbat: bq25730_async_rs::data_types::AdcVbat(0),
                    vsys: bq25730_async_rs::data_types::AdcVsys(0),
                    ichg: bq25730_async_rs::data_types::AdcIchg(0),
                    idchg: bq25730_async_rs::data_types::AdcIdchg(0),
                    iin: bq25730_async_rs::data_types::AdcIin::from_u8(0, false), // Use public constructor
                    psys: bq25730_async_rs::data_types::AdcPsys(0),
                    vbus: bq25730_async_rs::data_types::AdcVbus(0),
                    cmpin: bq25730_async_rs::data_types::AdcCmpin(0),
                }
            }),
        };
        bq25730_measurements_publisher.publish_immediate(bq25730_measurements_payload);

        Timer::after(Duration::from_secs(1)).await;
    }
}
