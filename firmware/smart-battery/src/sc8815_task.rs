use bq769x0_async_rs::registers::SysStatFlags;
use defmt::{debug, error, info, warn};
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

// Logging/diagnostics verbosity for SC8815 task
const ENABLE_SC8815_DIAG: bool = false;
const ENABLE_SC8815_SNAP: bool = false; // one-line snapshot each second

// Local alias for the concrete I2C device type used by this task.
type I2cDev = I2cDevice<
    'static,
    CriticalSectionRawMutex,
    I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
>;

// Session struct encapsulating an active SC8815 instance and related resources.
struct ScSession {
    sc: SC8815<I2cDev>,
    ce_pin: Output<'static>,
    pstop_pin: Output<'static>,
}

impl ScSession {
    async fn begin(
        mut ce_pin: Output<'static>,
        pstop_pin: Output<'static>,
        i2c: I2cDev,
        address: u8,
    ) -> Result<Self, (Output<'static>, Output<'static>, I2cDev)> {
        // Power-up sequence
        ce_pin.set_low();
        Timer::after(Duration::from_millis(100)).await;

        let mut sc = SC8815::new(i2c, address);
        debug!("sc:init");
        if let Err(_e) = sc.init().await {
            error!("sc_init_err");
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

        if let Err(_e) = sc.configure_device(&device_config).await {
            error!("sc_cfg_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(_e) = sc.set_vbat_monitor_ratio(0).await {
            error!("sc_vbat_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(_e) = sc.set_otg_mode(false).await {
            error!("sc_otg_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            warn!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(_e) = sc.set_adc_conversion(true).await {
            error!("sc_adc_start");
            let i2c_back = sc.release();
            ce_pin.set_high();
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if ENABLE_SC8815_DIAG {
            use sc8815::registers::Register as R;
            if let Ok(vbat_set) = sc.read_register(R::VbatSet).await {
                info!("SC cfg: VbatSet=0x{:02X}", vbat_set);
            }
            if let Ok(ratio) = sc.read_register(R::Ratio).await {
                info!("SC cfg: Ratio=0x{:02X}", ratio);
            }
            if let Ok((vbat_hi, vbat_lo)) = sc.read_consecutive_registers(R::VbatFbValue).await {
                info!("SC adc: VBAT_FB hi=0x{:02X} lo=0x{:02X}", vbat_hi, vbat_lo);
            }
            if let Ok(vinreg) = sc.read_register(R::VinregSet).await {
                info!("SC cfg: VinregSet=0x{:02X}", vinreg);
            }
            if let Ok(ibus_lim) = sc.read_register(R::IbusLimSet).await {
                info!("SC cfg: IbusLimSet=0x{:02X}", ibus_lim);
            }
            if let Ok(ibat_lim) = sc.read_register(R::IbatLimSet).await {
                info!("SC cfg: IbatLimSet=0x{:02X}", ibat_lim);
            }
            if let Ok(ctrl1) = sc.read_register(R::Ctrl1Set).await {
                info!("SC cfg: Ctrl1Set=0x{:02X}", ctrl1);
            }
            if let Ok(ctrl3) = sc.read_register(R::Ctrl3Set).await {
                info!("SC cfg: Ctrl3Set=0x{:02X}", ctrl3);
            }
            if let Ok(status) = sc.read_register(R::Status).await {
                info!("SC stat: Status=0x{:02X}", status);
            }
        }

        // Print compact configuration summary
        debug!(
            "sc:cfg v={} r1={} r2={} ib={} ibt={}",
            device_config.power.vinreg_voltage_mv,
            device_config.current_limits.rs1_mohm,
            device_config.current_limits.rs2_mohm,
            device_config.current_limits.ibus_limit_ma,
            device_config.current_limits.ibat_limit_ma
        );

        Ok(Self {
            sc,
            ce_pin,
            pstop_pin,
        })
    }

    async fn end(mut self) -> (Output<'static>, Output<'static>, I2cDev) {
        if let Err(_e) = self.sc.set_adc_conversion(false).await {
            error!("sc_adc_stop");
        }
        let i2c_back = self.sc.release();
        self.ce_pin.set_high();
        self.pstop_pin.set_high();
        (self.ce_pin, self.pstop_pin, i2c_back)
    }

    fn enable_power_stage(&mut self) {
        self.pstop_pin.set_low();
    }

    fn disable_power_stage(&mut self) {
        self.pstop_pin.set_high();
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
        I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
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
    let mut _adapter_present: bool = false;
    let mut adapter_holdoff_secs: u8 = 0; // debounce before allowing restart
    // OV cooldown (PSTOP gated, do not end session): 180 s
    let mut ov_pause_secs: u16 = 0;
    // Severe imbalance pause (Δ>=100 mV) clear when Δ<50 mV
    let mut imbalance_pause_active: bool = false;

    // Log de-noising latches
    let mut pol_start_latched: bool = false; // only log POL start on rising condition
    let mut last_pause_report: Option<(bool, bool)> = None; // (ov_pause_active, imbalance_pause_active)
    let mut last_probe_ac: Option<bool> = None; // quiesce probe AC edge log

    loop {
        // 全局“静默”策略：当 AC 不在时，尽量停靠会话并降低采样频率，
        // 但仍以低频探测适配器插入，避免进入不可唤醒的死锁。
        if crate::scheduler::is_quiesced() {
            if let Some(sess) = sc8815_session.take() {
                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                ce_pin_slot = Some(ce_back);
                pstop_pin_slot = Some(pstop_back);
                parked_i2c_device = Some(i2c_back);
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

            // 低频探测：尝试读取一次设备状态以判定适配器是否接入。
            // 不进入完整会话，不开启 ADC。为确保器件可响应 I2C，在探测窗口内短暂拉低 CE（启用芯片），
            // 但保持 PSTOP=High（功率级关闭），探测后立即恢复 CE=High。
            let mut status_for_pub = SC8815Status::default();
            // 瞬时上电芯片以便读状态
            if let Some(pin) = ce_pin_slot.as_mut() { pin.set_low(); }
            Timer::after(Duration::from_millis(20)).await;
            if let Some(dev) = parked_i2c_device.take() {
                let mut probe = SC8815::new(dev, address);
                match probe.get_device_status().await {
                    Ok(st) => {
                        status_for_pub = st;
                        // Edge-log AC presence to INFO for visibility during idle
                        if last_probe_ac.map(|v| v != st.ac_adapter_connected).unwrap_or(true) {
                            info!("sc:probe ac={}", st.ac_adapter_connected);
                            last_probe_ac = Some(st.ac_adapter_connected);
                        }
                    }
                    Err(_e) => {
                        // 保持默认
                        if last_probe_ac.map(|v| v != false).unwrap_or(true) {
                            info!("sc:probe err");
                            last_probe_ac = Some(false);
                        }
                    }
                }
                parked_i2c_device = Some(probe.release());
            }
            // 探测结束，关闭芯片以降功耗
            if let Some(pin) = ce_pin_slot.as_mut() { pin.set_high(); }

            let alerts_payload = Sc8815Alerts {
                device_status: status_for_pub,
                expected_charging: false,
                charging_confirmed: false,
                ov_pause_active: false,
                imbalance_pause_active: false,
            };
            sc8815_alerts_publisher.publish_immediate(alerts_payload);

            // 以较低频率轮询一次，留给全局状态聚合器时间吸收并唤醒调度器。
            Timer::after(Duration::from_millis(250)).await;
            continue;
        }
        if let Some(measurements) = bq76920_measurements_subscriber.try_next_message_pure() {
            latest_bq_measurements = Some(measurements);
        }

        if let Some(msg) = balancing_cv_sub.try_next_message_pure() {
            latest_bal_req = msg;
        }

        if let Some(bq_meas) = latest_bq_measurements.as_ref() {
            let pack_voltage_mv = bq_meas.core_measurements.total_voltage_mv;
            let system_status_flags = bq_meas.core_measurements.system_status.0;
            let _ov_fault = system_status_flags.contains(SysStatFlags::OV);
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
            } else if pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV {
                if latest_bal_req.require_cv {
                    debug!(
                        "cv_hold {}>={}",
                        pack_voltage_mv, PACK_CHARGE_STOP_THRESHOLD_MV
                    );
                } else {
                    debug!(
                        "pol:stop {}>={}",
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
                }
            } else if critical_fault {
                if charger_active {
                    warn!("blocking_fault {}", pack_voltage_mv);
                }
                // For ANY BQ critical fault (OV/UV/SCD/OCD), gate power stage and keep session for timed recovery
                if let Some(sess) = sc8815_session.as_mut() {
                    sess.disable_power_stage();
                } else if let Some(pin) = pstop_pin_slot.as_mut() {
                    pin.set_high();
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
                if ov_pause_secs == 0 {
                    ov_pause_secs = 180;
                    warn!("pause:ov 180s");
                }
            } else {
                // Maintain severe-imbalance pause state using spread; start by BalancingCvRequest.severe_imbalance
                if let Some(meas) = latest_bq_measurements.as_ref() {
                    let mut min_v = i32::MAX;
                    let mut max_v = i32::MIN;
                    for &v in meas.core_measurements.cell_voltages.voltages.iter() {
                        if v > 0 {
                            if v < min_v {
                                min_v = v;
                            }
                            if v > max_v {
                                max_v = v;
                            }
                        }
                    }
                    if max_v != i32::MIN && min_v != i32::MAX {
                        let spread = max_v - min_v;
                        if !imbalance_pause_active && latest_bal_req.severe_imbalance {
                            imbalance_pause_active = true;
                            warn!("pause:imb start");
                        }
                        if imbalance_pause_active && spread < 50 {
                            imbalance_pause_active = false;
                            debug!("pause:imb done");
                        }
                    }
                }

                // If in pause (OV or imbalance) and charger is active, gate power stage
                if (ov_pause_secs > 0 || imbalance_pause_active) && charger_active {
                    if let Some(sess) = sc8815_session.as_mut() {
                        sess.disable_power_stage();
                    }
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }

                // Charge start conditions (edge-logged)
                let pol_start_cond = pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV
                    && !charger_active
                    && adapter_holdoff_secs == 0;
                if pol_start_cond {
                    if !pol_start_latched {
                        debug!(
                            "pol+ {}<{}",
                            pack_voltage_mv, PACK_CHARGE_START_THRESHOLD_MV
                        );
                        pol_start_latched = true;
                    }
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
                                // Backoff to avoid rapid re-init thrash when init fails
                                if adapter_holdoff_secs < 5 {
                                    adapter_holdoff_secs = 5;
                                }
                                warn!("sc:backoff 5s");
                                continue;
                            }
                        }
                    }

                    // Now that the chip is configured and ADC running, (conditionally) enable power stage
                    if ov_pause_secs == 0 && !imbalance_pause_active {
                        if let Some(sess) = sc8815_session.as_mut() {
                            sess.enable_power_stage();
                        }
                        debug!("sc:g LL");
                        charger_active = true;
                        // leaving pause state → clear last pause report
                        last_pause_report = None;
                    } else {
                        if let Some(sess) = sc8815_session.as_mut() {
                            sess.disable_power_stage();
                        } else if let Some(pin) = pstop_pin_slot.as_mut() {
                            pin.set_high();
                        }
                        // log pause gating only when state (OV/IMB pair) changes
                        let pause_sig = (ov_pause_secs > 0, imbalance_pause_active);
                        if last_pause_report != Some(pause_sig) {
                            let mut b: u8 = 0; // bit0=LH, bit1=OV, bit2=IMB
                            b |= 1 << 0; // LH
                            if ov_pause_secs > 0 {
                                b |= 1 << 1;
                            }
                            if imbalance_pause_active {
                                b |= 1 << 2;
                            }
                            debug!("sc:g b=0x{:02x}", b);
                            last_pause_report = Some(pause_sig);
                        }
                        charger_active = false;
                    }
                } else {
                    // reset edge latch when the start condition is not true
                    pol_start_latched = false;
                }
            }
        }

        if sc8815_session.is_some() {
            let mut _latest_status_for_alerts: Option<SC8815Status> = None;
            match sc8815_session.as_mut() {
                Some(sess) => match sess.sc.get_device_status().await {
                    Ok(status) => {
                        let mut b: u8 = 0;
                        if status.ac_adapter_connected {
                            b |= 1 << 0;
                        }
                        if status.usb_load_detected {
                            b |= 1 << 1;
                        }
                        if status.otp_fault {
                            b |= 1 << 2;
                        }
                        if status.vbus_short_fault {
                            b |= 1 << 3;
                        }
                        debug!("sc:st b=0x{:02x}", b);

                        // Track adapter presence
                        if !status.ac_adapter_connected {
                            _adapter_present = false;
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
                            _latest_status_for_alerts = Some(status);
                            // Skip further work in this tick when adapter just lost
                            continue;
                        } else {
                            _adapter_present = true;
                        }

                        if status.otp_fault || status.vbus_short_fault {
                            warn!(
                                "sc:fault o={} v={}",
                                status.otp_fault, status.vbus_short_fault
                            );
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
                        _latest_status_for_alerts = Some(status);
                    }
                    Err(_e) => {
                        error!("sc:status!");
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
                        if ENABLE_SC8815_SNAP {
                            debug!(
                                "sc snap vbus={} vbat={} ibus={} ibat={}",
                                measurements.vbus_mv,
                                measurements.vbat_mv,
                                measurements.ibus_ma,
                                measurements.ibat_ma
                            );
                        }

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
                                info!("sc:chg_ok {}mA", ibat);
                            }

                            if charge_confirmed && drop_streak >= CHARGE_CONFIRMATION_SAMPLES {
                                charge_confirmed = false;
                                warn!("sc:chg_lost {}", ibat);
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
                    Err(_e) => {
                        error!("sc_adc_read!");
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                    }
                },
                None => {
                    // No session active; nothing to sample.
                }
            }

            if let Some(status) = _latest_status_for_alerts {
                let alerts_payload = Sc8815Alerts {
                    device_status: status,
                    expected_charging: charger_active,
                    charging_confirmed: charge_confirmed,
                    ov_pause_active: ov_pause_secs > 0,
                    imbalance_pause_active,
                };
                sc8815_alerts_publisher.publish_immediate(alerts_payload);
            }
        } else {
            // No session active: publish pause states as well
            let alerts_payload = Sc8815Alerts {
                device_status: SC8815Status::default(),
                expected_charging: charger_active,
                charging_confirmed: charge_confirmed,
                ov_pause_active: ov_pause_secs > 0,
                imbalance_pause_active,
            };
            sc8815_alerts_publisher.publish_immediate(alerts_payload);
        }

        if adapter_holdoff_secs > 0 {
            adapter_holdoff_secs = adapter_holdoff_secs.saturating_sub(1);
        }
        if ov_pause_secs > 0 {
            ov_pause_secs = ov_pause_secs.saturating_sub(1);
            if ov_pause_secs == 0 {
                debug!("pause:ov done");
            }
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}
