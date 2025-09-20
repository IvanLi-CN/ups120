use bq769x0_async_rs::registers::SysStatFlags;
use defmt::{error, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use sc8815::{
    DeadTime, DeviceConfiguration, OperatingMode, SC8815, SC8815Status, SwitchingFrequency,
};

use crate::data_types::BalancingCvRequest;
use crate::data_types::{Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    BalancingCvRequestSubscriber, Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher,
    Sc8815MeasurementsPublisher,
};

pub const SC8815_DEFAULT_ADDRESS: u8 = sc8815::registers::constants::DEFAULT_ADDRESS;

const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_OUTPUT_CUTOFF_THRESHOLD_MV: i32 = 12_500;
const MIN_EFFECTIVE_IBAT_MA: u16 = 100;
const IBAT_RELEASE_MARGIN_MA: u16 = 20;
const CHARGE_CONFIRMATION_SAMPLES: u8 = 3;

const ENABLE_SC8815_DIAG: bool = true;

// Local alias for the concrete I2C device type used by this task.
type I2cDev = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>;

// Session struct encapsulating an active SC8815 instance and related resources.
struct ScSession {
    sc: SC8815<I2cDev>,
    ce_pin: Output<'static>,
    pstop_pin: Output<'static>,
}

impl ScSession {
    async fn begin(
        mut ce_pin: Output<'static>,
        mut pstop_pin: Output<'static>,
        mut i2c: I2cDev,
        address: u8,
    ) -> Result<Self, (Output<'static>, Output<'static>, I2cDev)> {
        // Power-up sequence
        ce_pin.set_low();
        Timer::after(Duration::from_millis(100)).await;

        let mut sc = SC8815::new(i2c, address);
        info!("sc_session_init");
        if let Err(e) = sc.init().await {
            error!("sc_init_err {:?}", e);
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        // Per-session configuration
        let mut device_config = DeviceConfiguration::default();
        device_config.battery.use_internal_setting = false; // External divider
        device_config.current_limits.rs1_mohm = 10;
        device_config.current_limits.rs2_mohm = 10;
        // Bump limits per request: IBAT≈425 mA, IBUS=1000 mA for ample headroom
        // IBUS needs to cover power transfer from ~11.8V→~16.5V; 1 A 输入裕量充足
        device_config.current_limits.ibus_limit_ma = 1000;
        // Note: SC8815 quantizes IBAT in ~46.9mA steps (@12x, 10mΩ). 425mA → ~422mA effective.
        device_config.current_limits.ibat_limit_ma = 425;
        device_config.power.operating_mode = OperatingMode::Charging;
        device_config.power.switching_frequency = SwitchingFrequency::Freq450kHz;
        device_config.power.dead_time = DeadTime::Ns60;
        device_config.power.vinreg_voltage_mv = 11500;
        device_config.trickle_charging = true;
        device_config.charging_termination = true;
        device_config.use_ibus_for_charging = false;

        if let Err(e) = sc.configure_device(&device_config).await {
            error!("sc_cfg_err {:?}", e);
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(e) = sc.set_vbat_monitor_ratio(0).await {
            error!("sc_vbat_err {:?}", e);
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(e) = sc.set_otg_mode(false).await {
            error!("sc_otg_err {:?}", e);
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(e) = sc.set_adc_conversion(true).await {
            error!("sc_adc_start {:?}", e);
            let i2c_back = sc.release();
            ce_pin.set_high();
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if ENABLE_SC8815_DIAG {
            use sc8815::registers::Register as R;
            if let Ok(vbat_set) = sc.read_register(R::VbatSet).await {
                info!("diag_sc VbatSet=0x{:02X}", vbat_set);
            }
            if let Ok(ratio) = sc.read_register(R::Ratio).await {
                info!("diag_sc Ratio=0x{:02X}", ratio);
            }
            if let Ok((vbat_hi, vbat_lo)) = sc.read_consecutive_registers(R::VbatFbValue).await {
                info!("diag_sc VBAT_FB hi=0x{:02X} lo=0x{:02X}", vbat_hi, vbat_lo);
            }
            if let Ok(vinreg) = sc.read_register(R::VinregSet).await {
                info!("diag_sc VinregSet=0x{:02X}", vinreg);
            }
            if let Ok(ibus_lim) = sc.read_register(R::IbusLimSet).await {
                info!("diag_sc IbusLimSet=0x{:02X}", ibus_lim);
            }
            if let Ok(ibat_lim) = sc.read_register(R::IbatLimSet).await {
                info!("diag_sc IbatLimSet=0x{:02X}", ibat_lim);
            }
            if let Ok(ctrl1) = sc.read_register(R::Ctrl1Set).await {
                info!("diag_sc Ctrl1Set=0x{:02X}", ctrl1);
            }
            if let Ok(ctrl3) = sc.read_register(R::Ctrl3Set).await {
                info!("diag_sc Ctrl3Set=0x{:02X}", ctrl3);
            }
            if let Ok(status) = sc.read_register(R::Status).await {
                info!("diag_sc Status=0x{:02X}", status);
            }
        }

        Ok(Self {
            sc,
            ce_pin,
            pstop_pin,
        })
    }

    async fn end(mut self) -> (Output<'static>, Output<'static>, I2cDev) {
        if let Err(e) = self.sc.set_adc_conversion(false).await {
            error!("sc_adc_stop {:?}", e);
        }
        let i2c_back = self.sc.release();
        self.ce_pin.set_high();
        self.pstop_pin.set_high();
        (self.ce_pin, self.pstop_pin, i2c_back)
    }

    fn enable_power_stage(&mut self) {
        self.pstop_pin.set_low();
    }
}

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
    mut balancing_cv_sub: BalancingCvRequestSubscriber<'static>,
) {
    // Ensure charger is disabled until we explicitly start a session.
    ce_pin.set_high();
    pstop_pin.set_high();

    // Keep I2C device parked here; move into SC8815 only during an active session.
    let mut parked_i2c_device = Some(i2c_device);
    let mut sc8815_session: Option<ScSession> = None;
    // When no active session, pins stay here.
    let mut ce_pin_slot: Option<Output<'static>> = Some(ce_pin);
    let mut pstop_pin_slot: Option<Output<'static>> = Some(pstop_pin);

    let mut charger_active = false;
    let mut charge_confirmed = false;
    let mut confirm_streak: u8 = 0;
    let mut drop_streak: u8 = 0;
    let mut latest_bq_measurements = None;
    let mut latest_bal_req: BalancingCvRequest = BalancingCvRequest::default();
    let mut adapter_present: bool = false;
    let mut adapter_holdoff_secs: u8 = 0; // debounce before allowing restart

    loop {
        if let Some(measurements) = bq76920_measurements_subscriber.try_next_message_pure() {
            latest_bq_measurements = Some(measurements);
        }

        if let Some(msg) = balancing_cv_sub.try_next_message_pure() {
            latest_bal_req = msg;
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
                if sc8815_session.is_some() {
                    if let Some(sess) = sc8815_session.take() {
                        let (ce_back, pstop_back, i2c_back) = sess.end().await;
                        ce_pin_slot = Some(ce_back);
                        pstop_pin_slot = Some(pstop_back);
                        parked_i2c_device = Some(i2c_back);
                    }
                } else {
                    if let Some(pin) = pstop_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    if let Some(pin) = ce_pin_slot.as_mut() {
                        pin.set_high();
                    }
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV && !latest_bal_req.require_cv
            {
                info!(
                    "policy_stop {}>= {} mV",
                    pack_voltage_mv, PACK_CHARGE_STOP_THRESHOLD_MV
                );
                if sc8815_session.is_some() {
                    if let Some(sess) = sc8815_session.take() {
                        let (ce_back, pstop_back, i2c_back) = sess.end().await;
                        ce_pin_slot = Some(ce_back);
                        pstop_pin_slot = Some(pstop_back);
                        parked_i2c_device = Some(i2c_back);
                    }
                } else {
                    if let Some(pin) = pstop_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    if let Some(pin) = ce_pin_slot.as_mut() {
                        pin.set_high();
                    }
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if critical_fault {
                if charger_active {
                    warn!("blocking_fault {}", pack_voltage_mv);
                }
                if sc8815_session.is_some() {
                    if let Some(sess) = sc8815_session.take() {
                        let (ce_back, pstop_back, i2c_back) = sess.end().await;
                        ce_pin_slot = Some(ce_back);
                        pstop_pin_slot = Some(pstop_back);
                        parked_i2c_device = Some(i2c_back);
                    }
                } else {
                    if let Some(pin) = pstop_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    if let Some(pin) = ce_pin_slot.as_mut() {
                        pin.set_high();
                    }
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV
                && !charger_active
                && adapter_holdoff_secs == 0
            {
                info!(
                    "policy_start {}< {} mV",
                    pack_voltage_mv, PACK_CHARGE_START_THRESHOLD_MV
                );
                if sc8815_session.is_none() {
                    if let Some(pin) = pstop_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    // Move pins into the session
                    let ce_take = ce_pin_slot.take().expect("CE pin missing");
                    let pstop_take = pstop_pin_slot.take().expect("PSTOP pin missing");
                    let i2c_take = parked_i2c_device.take().expect("I2C missing");
                    match ScSession::begin(ce_take, pstop_take, i2c_take, address).await {
                        Ok(session) => {
                            sc8815_session = Some(session);
                        }
                        Err((ce_back, pstop_back, i2c_back)) => {
                            ce_pin_slot = Some(ce_back);
                            pstop_pin_slot = Some(pstop_back);
                            parked_i2c_device = Some(i2c_back);
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            continue;
                        }
                    }
                }

                // Now that the chip is configured and ADC running, enable power stage
                if let Some(sess) = sc8815_session.as_mut() {
                    sess.enable_power_stage();
                }
                info!("gates CE=LOW PSTOP=LOW (power stage enabled)");
                charger_active = true;
            }
        }

        if sc8815_session.is_some() {
            let mut latest_status_for_alerts: Option<SC8815Status> = None;
            match sc8815_session.as_mut() {
                Some(sess) => match sess.sc.get_device_status().await {
                    Ok(status) => {
                        info!(
                            "sc_stat {} {} {} {}",
                            status.ac_adapter_connected,
                            status.usb_load_detected,
                            status.otp_fault,
                            status.vbus_short_fault
                        );

                        // Track adapter presence
                        if !status.ac_adapter_connected {
                            adapter_present = false;
                            if let Some(sess) = sc8815_session.take() {
                                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                                ce_pin_slot = Some(ce_back);
                                pstop_pin_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                            }
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            adapter_holdoff_secs = adapter_holdoff_secs.max(5);
                            latest_status_for_alerts = Some(status);
                            // Skip further work in this tick when adapter just lost
                            continue;
                        } else {
                            adapter_present = true;
                        }

                        if status.otp_fault || status.vbus_short_fault {
                            warn!("sc_fault");
                            if let Some(sess) = sc8815_session.take() {
                                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                                ce_pin_slot = Some(ce_back);
                                pstop_pin_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                            }
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            continue;
                        }
                        latest_status_for_alerts = Some(status);
                    }
                    Err(e) => {
                        error!("Failed to read SC8815 status: {:?}", e);
                        if let Some(sess) = sc8815_session.take() {
                            let (ce_back, pstop_back, i2c_back) = sess.end().await;
                            ce_pin_slot = Some(ce_back);
                            pstop_pin_slot = Some(pstop_back);
                            parked_i2c_device = Some(i2c_back);
                        }
                        charger_active = false;
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                    }
                },
                None => {
                    // CE claimed enabled but no session exists; recover by disabling gates.
                    warn!("sc_session_missing");
                    if let Some(pin) = pstop_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    if let Some(pin) = ce_pin_slot.as_mut() {
                        pin.set_high();
                    }
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }
            }

            match sc8815_session.as_mut() {
                Some(sess) => match sess.sc.get_adc_measurements().await {
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
                },
                None => {
                    // No session active; nothing to sample.
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

        if adapter_holdoff_secs > 0 {
            adapter_holdoff_secs = adapter_holdoff_secs.saturating_sub(1);
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}
