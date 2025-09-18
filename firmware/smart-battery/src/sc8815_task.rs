use bq769x0_async_rs::registers::SysStatFlags;
use defmt::{error, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use sc8815::{
    DeadTime, DeviceConfiguration, OperatingMode, SC8815, SC8815Status, SwitchingFrequency,
};

use crate::data_types::{Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher, Sc8815MeasurementsPublisher,
};

pub const SC8815_DEFAULT_ADDRESS: u8 = sc8815::registers::constants::DEFAULT_ADDRESS;

const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_OUTPUT_CUTOFF_THRESHOLD_MV: i32 = 12_500;
const MIN_EFFECTIVE_IBAT_MA: u16 = 100;
const IBAT_RELEASE_MARGIN_MA: u16 = 20;
const CHARGE_CONFIRMATION_SAMPLES: u8 = 3;

/// Embassy task managing the SC8815 charger with safety gating.
#[embassy_executor::task]
pub async fn sc8815_task(
    mut ce_pin: Output<'static>,
    mut pstop_pin: Output<'static>,
    _exit_shipmode_pin: Output<'static>,
    i2c_device: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_stm32::mode::Async>,
    >,
    address: u8,
    sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
) {
    // Ensure charger is disabled until configuration completes.
    ce_pin.set_high();
    pstop_pin.set_high();

    Timer::after(Duration::from_millis(10)).await;
    ce_pin.set_low();
    Timer::after(Duration::from_millis(100)).await;
    pstop_pin.set_high();

    let mut sc8815 = SC8815::new(i2c_device, address);

    info!("sc_init");
    if let Err(e) = sc8815.init().await {
        error!("sc_init_err {:?}", e);
        ce_pin.set_high();
        warn!("sc_disable");
        return;
    }

    let mut device_config = DeviceConfiguration::default();
    device_config.battery.use_internal_setting = false; // External divider: Ru=140kΩ, Rd=10kΩ → ~18V target
    device_config.current_limits.rs1_mohm = 10;
    device_config.current_limits.rs2_mohm = 10;
    device_config.current_limits.ibus_limit_ma = 800;
    device_config.current_limits.ibat_limit_ma = 800;
    device_config.power.operating_mode = OperatingMode::Charging;
    device_config.power.switching_frequency = SwitchingFrequency::Freq450kHz;
    device_config.power.dead_time = DeadTime::Ns60;
    device_config.power.vinreg_voltage_mv = 11500;
    device_config.trickle_charging = true;
    device_config.charging_termination = true;
    device_config.use_ibus_for_charging = false;

    if let Err(e) = sc8815.configure_device(&device_config).await {
        error!("sc_cfg_err {:?}", e);
        ce_pin.set_high();
        warn!("sc_disable");
        return;
    }

    if let Err(e) = sc8815.set_vbat_monitor_ratio(0).await {
        error!("sc_vbat_err {:?}", e);
        ce_pin.set_high();
        warn!("sc_disable");
        return;
    }

    if let Err(e) = sc8815.set_otg_mode(false).await {
        error!("sc_otg_err {:?}", e);
        ce_pin.set_high();
        warn!("sc_disable");
        return;
    }

    if let Err(e) = sc8815.set_adc_conversion(true).await {
        error!("sc_adc_err {:?}", e);
        ce_pin.set_high();
        warn!("sc_disable");
        return;
    }

    // Default to CE high once configuration completes; SC8815 stays idle until needed.
    ce_pin.set_high();

    let mut ce_enabled = false;
    let mut charger_active = false;
    let mut charge_confirmed = false;
    let mut confirm_streak: u8 = 0;
    let mut drop_streak: u8 = 0;
    let mut latest_bq_measurements = None;

    loop {
        if let Some(measurements) = bq76920_measurements_subscriber.try_next_message_pure() {
            latest_bq_measurements = Some(measurements);
        }

        if let Some(bq_meas) = latest_bq_measurements.as_ref() {
            let pack_voltage_mv = bq_meas.core_measurements.total_voltage_mv;
            let system_status_flags = bq_meas.core_measurements.system_status.0;
            let critical_fault = system_status_flags.intersects(
                SysStatFlags::OV | SysStatFlags::UV | SysStatFlags::SCD | SysStatFlags::OCD,
            );

            if pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV {
                if charger_active {
                    warn!("cutoff {}", pack_voltage_mv);
                }
                pstop_pin.set_high();
                if ce_enabled {
                    if let Err(e) = sc8815.set_adc_conversion(false).await {
                        error!("sc_adc_stop {:?}", e);
                    }
                    ce_pin.set_high();
                    ce_enabled = false;
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV {
                pstop_pin.set_high();
                if ce_enabled {
                    if let Err(e) = sc8815.set_adc_conversion(false).await {
                        error!("sc_adc_stop {:?}", e);
                    }
                    ce_pin.set_high();
                    ce_enabled = false;
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if critical_fault {
                if charger_active {
                    warn!("blocking_fault {}", pack_voltage_mv);
                }
                pstop_pin.set_high();
                if ce_enabled {
                    if let Err(e) = sc8815.set_adc_conversion(false).await {
                        error!("sc_adc_stop {:?}", e);
                    }
                    ce_pin.set_high();
                    ce_enabled = false;
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV && !charger_active {
                pstop_pin.set_high();
                if !ce_enabled {
                    ce_pin.set_low();
                    Timer::after(Duration::from_millis(100)).await;
                    if let Err(e) = sc8815.set_adc_conversion(true).await {
                        error!("sc_adc_start {:?}", e);
                        ce_pin.set_high();
                        charger_active = false;
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                        continue;
                    }
                    ce_enabled = true;
                }
                pstop_pin.set_low();
                charger_active = true;
            }
        }

        if ce_enabled {
            let mut latest_status_for_alerts: Option<SC8815Status> = None;

            match sc8815.get_device_status().await {
                Ok(status) => {
                    info!(
                        "sc_stat {} {} {} {}",
                        status.ac_adapter_connected,
                        status.usb_load_detected,
                        status.otp_fault,
                        status.vbus_short_fault
                    );

                    if status.otp_fault || status.vbus_short_fault {
                        warn!("sc_fault");
                        pstop_pin.set_high();
                        if let Err(e) = sc8815.set_adc_conversion(false).await {
                            error!("sc_adc_stop {:?}", e);
                        }
                        ce_pin.set_high();
                        ce_enabled = false;
                        charger_active = false;
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                    }
                    latest_status_for_alerts = Some(status);
                }
                Err(e) => {
                    error!("Failed to read SC8815 status: {:?}", e);
                    pstop_pin.set_high();
                    if let Err(e) = sc8815.set_adc_conversion(false).await {
                        error!("sc_adc_stop {:?}", e);
                    }
                    ce_pin.set_high();
                    ce_enabled = false;
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }
            }

            match sc8815.get_adc_measurements().await {
                Ok(measurements) => {
                    info!(
                        "VBUS={}mV VBAT={}mV IBUS={}mA IBAT={}mA",
                        measurements.vbus_mv,
                        measurements.vbat_mv,
                        measurements.ibus_ma,
                        measurements.ibat_ma
                    );

                    if charger_active {
                        let ibat = measurements.ibat_ma;
                        if ibat >= MIN_EFFECTIVE_IBAT_MA {
                            if confirm_streak < CHARGE_CONFIRMATION_SAMPLES {
                                confirm_streak += 1;
                            }
                            drop_streak = 0;
                        } else if ibat
                            <= MIN_EFFECTIVE_IBAT_MA.saturating_sub(IBAT_RELEASE_MARGIN_MA)
                        {
                            if drop_streak < CHARGE_CONFIRMATION_SAMPLES {
                                drop_streak += 1;
                            }
                            confirm_streak = 0;
                        }

                        if !charge_confirmed && confirm_streak >= CHARGE_CONFIRMATION_SAMPLES {
                            charge_confirmed = true;
                            info!("sc_charge_confirmed {}", ibat);
                        }

                        if charge_confirmed && drop_streak >= CHARGE_CONFIRMATION_SAMPLES {
                            charge_confirmed = false;
                            warn!("sc_charge_lost {}", ibat);
                        }
                    } else {
                        confirm_streak = 0;
                        drop_streak = 0;
                        charge_confirmed = false;
                    }

                    let meas_payload = Sc8815Measurements {
                        adc_measurements: measurements,
                    };
                    sc8815_measurements_publisher.publish_immediate(meas_payload);
                }
                Err(e) => {
                    error!("sc_adc_read {:?}", e);
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }
            }

            if let Some(status) = latest_status_for_alerts {
                let alerts_payload = Sc8815Alerts {
                    device_status: status,
                    expected_charging: charger_active,
                    charging_confirmed: charge_confirmed,
                };
                sc8815_alerts_publisher.publish_immediate(alerts_payload);
            }
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
