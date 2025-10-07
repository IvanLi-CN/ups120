use bq769x0_async_rs::registers::SysStatFlags;
use defmt::{debug, error, info};
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
use crate::state_bits::{self, bits as sbits};
use embassy_stm32::exti::ExtiInput;
use portable_atomic::AtomicBool;

pub const SC8815_DEFAULT_ADDRESS: u8 = sc8815::registers::constants::DEFAULT_ADDRESS;

const PACK_CHARGE_START_THRESHOLD_MV: i32 = 17_000;
const PACK_CHARGE_STOP_THRESHOLD_MV: i32 = 18_500;
const PACK_OUTPUT_CUTOFF_THRESHOLD_MV: i32 = 12_500;
const MIN_EFFECTIVE_IBAT_MA: u16 = 100;
const IBAT_RELEASE_MARGIN_MA: u16 = 20;
const CHARGE_CONFIRMATION_SAMPLES: u8 = 3;
const ITERM_EXIT_MULTIPLIER_X10: u16 = 12;
const FULL_ENTER_SECS: u32 = 60;
const FULL_EXIT_SECS: u32 = 10;

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
    ce_ctl: Output<'static>,
    pstop_ctl: Output<'static>,
}

// SC8815 INT → 事件通知（EXTI 边沿触发后唤醒本任务查询状态）
static SC_INT_PENDING: AtomicBool = AtomicBool::new(false);

#[embassy_executor::task]
pub async fn sc8815_irq_task(mut int_pin: ExtiInput<'static>) {
    loop {
        int_pin.wait_for_falling_edge().await;
        SC_INT_PENDING.store(true, portable_atomic::Ordering::Relaxed);
        crate::sleep_manager::bump("sc-int");
    }
}

impl ScSession {
    async fn begin(
        mut ce_pin: Output<'static>,
        pstop_pin: Output<'static>,
        i2c: I2cDev,
        address: u8,
    ) -> Result<Self, (Output<'static>, Output<'static>, I2cDev)> {
        // Power-up sequence: CE_CTL=High (chip CE=Enable via inversion)
        ce_pin.set_high();
        Timer::after(Duration::from_millis(100)).await;

        let mut sc = SC8815::new(i2c, address);
        // init
        if let Err(_e) = sc.init().await {
            error!("sc_init_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            defmt::debug!("sc_disable");
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
            defmt::debug!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(_e) = sc.set_vbat_monitor_ratio(0).await {
            error!("sc_vbat_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            defmt::debug!("sc_disable");
            return Err((ce_pin, pstop_pin, i2c_back));
        }

        if let Err(_e) = sc.set_otg_mode(false).await {
            error!("sc_otg_err");
            let i2c_back = sc.release();
            ce_pin.set_high();
            defmt::debug!("sc_disable");
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
                debug!("SC cfg: VbatSet=0x{:02X}", vbat_set);
            }
            if let Ok(ratio) = sc.read_register(R::Ratio).await {
                debug!("SC cfg: Ratio=0x{:02X}", ratio);
            }
            if let Ok((vbat_hi, vbat_lo)) = sc.read_consecutive_registers(R::VbatFbValue).await {
                debug!("SC adc: VBAT_FB hi=0x{:02X} lo=0x{:02X}", vbat_hi, vbat_lo);
            }
            if let Ok(vinreg) = sc.read_register(R::VinregSet).await {
                debug!("SC cfg: VinregSet=0x{:02X}", vinreg);
            }
            if let Ok(ibus_lim) = sc.read_register(R::IbusLimSet).await {
                debug!("SC cfg: IbusLimSet=0x{:02X}", ibus_lim);
            }
            if let Ok(ibat_lim) = sc.read_register(R::IbatLimSet).await {
                debug!("SC cfg: IbatLimSet=0x{:02X}", ibat_lim);
            }
            if let Ok(ctrl1) = sc.read_register(R::Ctrl1Set).await {
                debug!("SC cfg: Ctrl1Set=0x{:02X}", ctrl1);
            }
            if let Ok(ctrl3) = sc.read_register(R::Ctrl3Set).await {
                debug!("SC cfg: Ctrl3Set=0x{:02X}", ctrl3);
            }
            if let Ok(status) = sc.read_register(R::Status).await {
                debug!("SC stat: Status=0x{:02X}", status);
            }
        }

        // Print compact configuration summary
        // cfg summary

        Ok(Self {
            sc,
            ce_ctl: ce_pin,
            pstop_ctl: pstop_pin,
        })
    }

    async fn end(mut self) -> (Output<'static>, Output<'static>, I2cDev) {
        if let Err(_e) = self.sc.set_adc_conversion(false).await {
            error!("sc_adc_stop");
        }
        let i2c_back = self.sc.release();
        // Keep charger disabled and power stage stopped on session end
        self.ce_ctl.set_low();
        self.pstop_ctl.set_low();
        (self.ce_ctl, self.pstop_ctl, i2c_back)
    }

    fn enable_power_stage(&mut self) {
        // Allow power stage (inverted control): PSTOP_CTL=High → chip PSTOP=Low
        self.pstop_ctl.set_high();
    }

    fn disable_power_stage(&mut self) {
        // Stop power stage: PSTOP_CTL=Low → chip PSTOP=High
        self.pstop_ctl.set_low();
    }
}

#[derive(Copy, Clone)]
struct StateFlagsCtx {
    ac_present: bool,
    sc_active: bool,
    charger_active: bool,
    charge_confirmed: bool,
    pause_active: bool,
    imbalance_pause_active: bool,
    full_latched: bool,
    sc_fault: bool,
}

pub struct Sc8815TaskArgs {
    pub ce_ctl: Output<'static>,
    pub pstop_ctl: Output<'static>,
    pub i2c_device: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
    >,
    pub address: u8,
    pub sc8815_alerts_publisher: Sc8815AlertsPublisher<'static>,
    pub sc8815_measurements_publisher: Sc8815MeasurementsPublisher<'static>,
    pub bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
    pub balancing_cv_sub: BalancingCvRequestSubscriber<'static>,
}

#[inline(always)]
fn refresh_state_flags(ctx: StateFlagsCtx) {
    let paused = ctx.ac_present && (ctx.pause_active || ctx.imbalance_pause_active);
    let charging = ctx.charger_active || ctx.charge_confirmed || paused;
    const MASK: u16 = sbits::AC_PRESENT
        | sbits::CHARGING
        | sbits::CHG_PAUSED
        | sbits::FULL
        | sbits::FAULT_SC
        | sbits::ACTIVE_SC;
    let mut value: u16 = 0;
    if ctx.ac_present {
        value |= sbits::AC_PRESENT;
    }
    if charging {
        value |= sbits::CHARGING;
    }
    if paused {
        value |= sbits::CHG_PAUSED;
    }
    if ctx.full_latched {
        value |= sbits::FULL;
    }
    if ctx.sc_fault {
        value |= sbits::FAULT_SC;
    }
    if ctx.sc_active {
        value |= sbits::ACTIVE_SC;
    }
    state_bits::update_flags(MASK, value);
}

/// Embassy task managing the SC8815 charger with safety gating.
#[embassy_executor::task]
pub async fn sc8815_task(args: Sc8815TaskArgs) {
    let Sc8815TaskArgs {
        mut ce_ctl,
        mut pstop_ctl,
        i2c_device,
        address,
        sc8815_alerts_publisher,
        sc8815_measurements_publisher,
        mut bq76920_measurements_subscriber,
        mut balancing_cv_sub,
    } = args;
    // Ensure charger is disabled and power stage stopped before any session.
    // Inverted control (MOSFET): CE_CTL High=enable, Low=disable; PSTOP_CTL Low=stop.
    ce_ctl.set_low();
    pstop_ctl.set_low();
    info!("pstop_ctl=L (stop) at init");

    // Keep I2C device parked here; move into SC8815 only during an active session.
    let mut parked_i2c_device = Some(i2c_device);
    let mut sc8815_session: Option<ScSession> = None;
    // When no active session, pins stay here.
    let mut ce_ctl_slot: Option<Output<'static>> = Some(ce_ctl);
    let mut pstop_ctl_slot: Option<Output<'static>> = Some(pstop_ctl);

    let mut charger_active = false;
    let mut charge_confirmed = false;
    let mut confirm_streak: u8 = 0;
    let mut drop_streak: u8 = 0;
    let mut latest_bq_measurements = None;
    let mut latest_bal_req: BalancingCvRequest = BalancingCvRequest::default();
    let mut _adapter_present: bool = false;
    let mut adapter_holdoff_secs: u8 = 0; // debounce before allowing restart
    // Cooldowns (seconds): OV=180s, UV=10s, OC(OCD/SCD)=30s
    let mut ov_pause_secs: u16 = 0;
    let mut uv_pause_secs: u16 = 0;
    let mut oc_pause_secs: u16 = 0;
    // Severe imbalance pause (Δ>=100 mV) clear when Δ<50 mV
    let mut imbalance_pause_active: bool = false;
    // 来自 BQ 的温度暂停请求（通过 BalancingCvRequest 下发）
    let mut temp_pause_cmd: bool = false;
    // 100ms tick accumulator → drive *_pause_secs at 1 Hz
    let mut tick_100ms: u8 = 0;
    let mut sc_comm_fail_streak: u8 = 0;
    let mut sc_fault_flag: bool = false;
    let mut last_status_eoc: bool = false;
    let mut full_enter_ms: u32 = 0;
    let mut full_exit_ms: u32 = 0;
    let mut full_latched: bool = false;

    // Log de-noising latch for pause state changes only
    let mut last_pause_report: Option<(bool, bool)> = None; // (ov_pause_active, imbalance_pause_active)
    // quiesce INT mode: no probe state needed
    // dropout counters omitted in this step to keep flash within limits
    // Boot probe: unconditionally attempt to initialize SC8815 once at power-up
    if sc8815_session.is_none()
        && let (Some(ce_tmp), Some(pstop_tmp)) = (ce_ctl_slot.take(), pstop_ctl_slot.take())
    {
        // Ensure power stage is stopped during probe
        let mut pstop_tmp = pstop_tmp;
        pstop_tmp.set_low();
        match ScSession::begin(
            ce_tmp,
            pstop_tmp,
            parked_i2c_device.take().expect("I2C missing"),
            address,
        )
        .await
        {
            Ok(session) => {
                sc8815_session = Some(session);
                info!("sc:probe ok");
                crate::failsafe::set_sc_online(true);
            }
            Err((ce_back, pstop_back, i2c_back)) => {
                ce_ctl_slot = Some(ce_back);
                pstop_ctl_slot = Some(pstop_back);
                parked_i2c_device = Some(i2c_back);
                info!("sc:probe fail");
                // keep offline until a later success
                crate::failsafe::set_sc_online(false);
            }
        }
    }

    loop {
        // 全局 fail-safe：一旦请求，强制功率级停机（PSTOP=高），直到 BQ 成功通信后清除
        if crate::failsafe::is_pstop_requested() {
            if let Some(sess) = sc8815_session.as_mut() {
                sess.disable_power_stage();
            } else if let Some(pin) = pstop_ctl_slot.as_mut() {
                pin.set_low();
            }
            charger_active = false;
            charge_confirmed = false;
        }
        // 全局“静默”策略：当 AC 不在时，停靠会话并仅依赖 INT 事件，不再轮询。
        if crate::failsafe::is_quiesced() {
            if let Some(sess) = sc8815_session.take() {
                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                ce_ctl_slot = Some(ce_back);
                pstop_ctl_slot = Some(pstop_back);
                parked_i2c_device = Some(i2c_back);
            } else {
                if let Some(pin) = pstop_ctl_slot.as_mut() {
                    pin.set_low();
                }
                if let Some(pin) = ce_ctl_slot.as_mut() {
                    pin.set_high(); // 芯片在线，以便产生 INT（反相控制）
                }
            }
            charger_active = false;
            charge_confirmed = false;
            confirm_streak = 0;
            drop_streak = 0;
            _adapter_present = false;
            full_latched = false;
            full_enter_ms = 0;
            full_exit_ms = 0;
            refresh_state_flags(StateFlagsCtx {
                ac_present: false,
                sc_active: false,
                charger_active,
                charge_confirmed,
                pause_active: (ov_pause_secs > 0 || uv_pause_secs > 0 || oc_pause_secs > 0)
                    || temp_pause_cmd,
                imbalance_pause_active,
                full_latched,
                sc_fault: sc_fault_flag,
            });
            // 短等待：允许由 IRQ 唤醒，无状态轮询
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }
        if let Some(measurements) = bq76920_measurements_subscriber.try_next_message_pure() {
            latest_bq_measurements = Some(measurements);
        }

        if let Some(msg) = balancing_cv_sub.try_next_message_pure() {
            latest_bal_req = msg;
            temp_pause_cmd = msg.temp_pause;
        }

        if let Some(bq_meas) = latest_bq_measurements.as_ref() {
            let pack_voltage_mv = bq_meas.core_measurements.total_voltage_mv;
            let system_status_flags = bq_meas.core_measurements.system_status.0;
            let ov_fault = system_status_flags.contains(SysStatFlags::OV);
            let uv_fault = system_status_flags.contains(SysStatFlags::UV);
            let scd_fault = system_status_flags.contains(SysStatFlags::SCD);
            let ocd_fault = system_status_flags.contains(SysStatFlags::OCD);
            let critical_fault = ov_fault || uv_fault || scd_fault || ocd_fault;

            if pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV {
                if charger_active {
                    defmt::info!("cutoff {}", pack_voltage_mv);
                }
                if sc8815_session.is_some() {
                    if let Some(sess) = sc8815_session.take() {
                        let (ce_back, pstop_back, i2c_back) = sess.end().await;
                        ce_ctl_slot = Some(ce_back);
                        pstop_ctl_slot = Some(pstop_back);
                        parked_i2c_device = Some(i2c_back);
                    }
                } else {
                    if let Some(pin) = pstop_ctl_slot.as_mut() {
                        pin.set_low();
                    }
                    if let Some(pin) = ce_ctl_slot.as_mut() {
                        pin.set_low();
                    }
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV {
                if !latest_bal_req.require_cv {
                    info!(
                        "stop vb>{} {}",
                        PACK_CHARGE_STOP_THRESHOLD_MV, pack_voltage_mv
                    );
                    if sc8815_session.is_some() {
                        if let Some(sess) = sc8815_session.take() {
                            let (ce_back, pstop_back, i2c_back) = sess.end().await;
                            ce_ctl_slot = Some(ce_back);
                            pstop_ctl_slot = Some(pstop_back);
                            parked_i2c_device = Some(i2c_back);
                        }
                    } else {
                        if let Some(pin) = pstop_ctl_slot.as_mut() {
                            pin.set_low();
                        }
                        if let Some(pin) = ce_ctl_slot.as_mut() {
                            pin.set_low();
                        }
                    }
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }
            } else if critical_fault {
                if charger_active {
                    defmt::info!("blocking_fault {}", pack_voltage_mv);
                }
                // 打印阻断充电的故障原因
                defmt::info!(
                    "blk f=0x{:02x} vb={}",
                    system_status_flags.bits(),
                    pack_voltage_mv
                );
                // 针对不同故障设置不同冷却时间：OV 180s、UV 10s、OCD/SCD 30s
                if ov_fault {
                    ov_pause_secs = ov_pause_secs.max(180);
                }
                if uv_fault {
                    uv_pause_secs = uv_pause_secs.max(10);
                }
                if scd_fault || ocd_fault {
                    oc_pause_secs = oc_pause_secs.max(30);
                }
                // 对任一故障：功率级停机，保持会话等待恢复
                if let Some(sess) = sc8815_session.as_mut() {
                    sess.disable_power_stage();
                } else if let Some(pin) = pstop_ctl_slot.as_mut() {
                    pin.set_high();
                }
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
                if ov_pause_secs == 0 {
                    ov_pause_secs = 180;
                    // ov cooldown start
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
                            defmt::info!("pause:imb start");
                        }
                        if imbalance_pause_active && spread < 50 {
                            imbalance_pause_active = false;
                            // imb pause done
                        }
                    }
                }

                // If in pause (OV or imbalance) and charger is active, gate power stage
                if ((ov_pause_secs > 0 || imbalance_pause_active) || temp_pause_cmd)
                    && charger_active
                {
                    if let Some(sess) = sc8815_session.as_mut() {
                        sess.disable_power_stage();
                    }
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }

                // Charge start conditions (edge-logged)
                // 当处于温度暂停时，严格禁止进入启动路径，避免启停抖动与日志刷屏
                let pol_start_cond = pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV
                    && !charger_active
                    && adapter_holdoff_secs == 0
                    && !temp_pause_cmd;
                if pol_start_cond {
                    // (edge latch removed)
                    if sc8815_session.is_none() {
                        if let Some(pin) = pstop_ctl_slot.as_mut() {
                            pin.set_low();
                        }
                        // Move pins into the session
                        let ce_take = ce_ctl_slot.take().expect("CE pin missing");
                        let pstop_take = pstop_ctl_slot.take().expect("PSTOP pin missing");
                        let i2c_take = parked_i2c_device.take().expect("I2C missing");
                        match ScSession::begin(ce_take, pstop_take, i2c_take, address).await {
                            Ok(session) => {
                                sc8815_session = Some(session);
                            }
                            Err((ce_back, pstop_back, i2c_back)) => {
                                ce_ctl_slot = Some(ce_back);
                                pstop_ctl_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                                charger_active = false;
                                charge_confirmed = false;
                                confirm_streak = 0;
                                drop_streak = 0;
                                // Backoff to avoid rapid re-init thrash when init fails
                                if adapter_holdoff_secs < 5 {
                                    adapter_holdoff_secs = 5;
                                }
                                defmt::info!("sc:backoff 5s");
                                continue;
                            }
                        }
                    }

                    // Now that the chip is configured and ADC running, (conditionally) enable power stage
                    if ov_pause_secs == 0
                        && uv_pause_secs == 0
                        && oc_pause_secs == 0
                        && !imbalance_pause_active
                        && !temp_pause_cmd
                    {
                        if let Some(sess) = sc8815_session.as_mut() {
                            sess.enable_power_stage();
                        }
                        info!("start vb={}", pack_voltage_mv);
                        charger_active = true;
                        // leaving pause state → clear last pause report
                        last_pause_report = None;
                    } else {
                        if let Some(sess) = sc8815_session.as_mut() {
                            sess.disable_power_stage();
                        } else if let Some(pin) = pstop_ctl_slot.as_mut() {
                            pin.set_low();
                        }
                        // log pause gating only when state (OV/IMB pair) changes
                        let pause_sig = (
                            (ov_pause_secs > 0 || uv_pause_secs > 0 || oc_pause_secs > 0),
                            imbalance_pause_active,
                        );
                        if last_pause_report != Some(pause_sig) {
                            let mut b: u8 = 0; // bit0=LH, bit1=OV, bit2=IMB
                            b |= 1 << 0; // LH
                            if ov_pause_secs > 0 || uv_pause_secs > 0 || oc_pause_secs > 0 {
                                b |= 1 << 1;
                            }
                            if imbalance_pause_active {
                                b |= 1 << 2;
                            }
                            let _ = b; // shrink log
                            last_pause_report = Some(pause_sig);
                        }
                        charger_active = false;
                    }
                }
            }
        }

        if sc8815_session.is_some() {
            let mut _latest_status_for_alerts: Option<SC8815Status> = None;
            match sc8815_session.as_mut() {
                Some(sess) => match sess.sc.get_device_status().await {
                    Ok(status) => {
                        sc_fault_flag = status.otp_fault || status.vbus_short_fault;
                        last_status_eoc = status.eoc;
                        // status fetched
                        // 更新 AC 静默策略
                        crate::failsafe::set_ac_present(status.ac_adapter_connected);
                        crate::failsafe::set_sc_online(true);
                        sc_comm_fail_streak = 0;
                        let now_ms = embassy_time::Instant::now().as_millis() as u32;
                        crate::failsafe::sc_heartbeat_update(now_ms);
                        // ok

                        // Track adapter presence
                        if !status.ac_adapter_connected {
                            _adapter_present = false;
                            if let Some(sess) = sc8815_session.take() {
                                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                                ce_ctl_slot = Some(ce_back);
                                pstop_ctl_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                            }
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            adapter_holdoff_secs = adapter_holdoff_secs.max(5);
                            full_latched = false;
                            full_enter_ms = 0;
                            full_exit_ms = 0;
                            refresh_state_flags(StateFlagsCtx {
                                ac_present: false,
                                sc_active: false,
                                charger_active,
                                charge_confirmed,
                                pause_active: (ov_pause_secs > 0
                                    || uv_pause_secs > 0
                                    || oc_pause_secs > 0)
                                    || temp_pause_cmd,
                                imbalance_pause_active,
                                full_latched,
                                sc_fault: sc_fault_flag,
                            });
                            _latest_status_for_alerts = Some(status);
                            // Skip further work in this tick when adapter just lost
                            continue;
                        } else {
                            _adapter_present = true;
                        }

                        // EOC: terminate charging session immediately per device indication
                        if status.eoc {
                            info!("eoc");
                            if let Some(sess) = sc8815_session.take() {
                                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                                ce_ctl_slot = Some(ce_back);
                                pstop_ctl_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                            }
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            full_latched = true;
                            full_exit_ms = 0;
                            full_enter_ms = FULL_ENTER_SECS * 1000;
                            refresh_state_flags(StateFlagsCtx {
                                ac_present: _adapter_present,
                                sc_active: false,
                                charger_active,
                                charge_confirmed,
                                pause_active: ov_pause_secs > 0
                                    || uv_pause_secs > 0
                                    || oc_pause_secs > 0,
                                imbalance_pause_active,
                                full_latched,
                                sc_fault: sc_fault_flag,
                            });
                            _latest_status_for_alerts = Some(status);
                            continue;
                        }

                        if status.otp_fault || status.vbus_short_fault {
                            defmt::info!(
                                "sc:fault o={} v={}",
                                status.otp_fault,
                                status.vbus_short_fault
                            );
                            if let Some(sess) = sc8815_session.take() {
                                let (ce_back, pstop_back, i2c_back) = sess.end().await;
                                ce_ctl_slot = Some(ce_back);
                                pstop_ctl_slot = Some(pstop_back);
                                parked_i2c_device = Some(i2c_back);
                            }
                            charger_active = false;
                            charge_confirmed = false;
                            confirm_streak = 0;
                            drop_streak = 0;
                            refresh_state_flags(StateFlagsCtx {
                                ac_present: _adapter_present,
                                sc_active: false,
                                charger_active,
                                charge_confirmed,
                                pause_active: ov_pause_secs > 0
                                    || uv_pause_secs > 0
                                    || oc_pause_secs > 0,
                                imbalance_pause_active,
                                full_latched,
                                sc_fault: sc_fault_flag,
                            });
                            continue;
                        }
                        _latest_status_for_alerts = Some(status);
                    }
                    Err(_e) => {
                        error!("sc:status!");
                        // err
                        if let Some(sess) = sc8815_session.take() {
                            let (ce_back, pstop_back, i2c_back) = sess.end().await;
                            ce_ctl_slot = Some(ce_back);
                            pstop_ctl_slot = Some(pstop_back);
                            parked_i2c_device = Some(i2c_back);
                        }
                        charger_active = false;
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                        sc_comm_fail_streak = sc_comm_fail_streak.saturating_add(1);
                        if sc_comm_fail_streak >= 3 {
                            crate::failsafe::set_sc_online(false);
                        }
                    }
                },
                None => {
                    // CE claimed enabled but no session exists; recover by disabling gates.
                    defmt::info!("sc_session_missing");
                    if let Some(pin) = pstop_ctl_slot.as_mut() {
                        pin.set_low();
                    }
                    if let Some(pin) = ce_ctl_slot.as_mut() {
                        pin.set_low();
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
                            // snap
                        }
                        let now_ms = embassy_time::Instant::now().as_millis() as u32;
                        crate::failsafe::sc_heartbeat_update(now_ms);
                        crate::failsafe::set_sc_online(true);
                        sc_comm_fail_streak = 0;
                        // ok

                        let last_vbat_mv = measurements.vbat_mv as i32;
                        if _adapter_present {
                            if last_status_eoc {
                                full_latched = true;
                                full_exit_ms = 0;
                            }
                            let enter_ok = last_vbat_mv >= PACK_CHARGE_STOP_THRESHOLD_MV
                                && measurements.ibat_ma <= MIN_EFFECTIVE_IBAT_MA;
                            if !full_latched {
                                if enter_ok {
                                    full_enter_ms =
                                        (full_enter_ms + 100).min((FULL_ENTER_SECS + 1) * 1000);
                                } else {
                                    full_enter_ms = 0;
                                }
                                if full_enter_ms >= FULL_ENTER_SECS * 1000 {
                                    full_latched = true;
                                    full_exit_ms = 0;
                                }
                            } else {
                                let exit_current_threshold = (MIN_EFFECTIVE_IBAT_MA as u32)
                                    .saturating_mul(ITERM_EXIT_MULTIPLIER_X10 as u32)
                                    .div_ceil(10)
                                    as u16;
                                let exit_by_current =
                                    measurements.ibat_ma >= exit_current_threshold;
                                let exit_by_voltage = last_vbat_mv < PACK_CHARGE_START_THRESHOLD_MV;
                                let pause_active =
                                    (ov_pause_secs > 0 || uv_pause_secs > 0 || oc_pause_secs > 0)
                                        || imbalance_pause_active;
                                let charging_flags = charger_active
                                    || charge_confirmed
                                    || (pause_active && _adapter_present);
                                if exit_by_current || exit_by_voltage || !charging_flags {
                                    full_exit_ms =
                                        (full_exit_ms + 100).min((FULL_EXIT_SECS + 1) * 1000);
                                } else {
                                    full_exit_ms = 0;
                                }
                                if full_exit_ms >= FULL_EXIT_SECS * 1000 {
                                    full_latched = false;
                                    full_enter_ms = 0;
                                }
                            }
                        } else {
                            full_latched = false;
                            full_enter_ms = 0;
                            full_exit_ms = 0;
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
                                defmt::info!("sc:chg_lost {}", ibat);
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
                        // err
                        charge_confirmed = false;
                        confirm_streak = 0;
                        drop_streak = 0;
                        sc_comm_fail_streak = sc_comm_fail_streak.saturating_add(1);
                        if sc_comm_fail_streak >= 3 {
                            crate::failsafe::set_sc_online(false);
                        }
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
                    ov_pause_active: (ov_pause_secs > 0)
                        || (uv_pause_secs > 0)
                        || (oc_pause_secs > 0),
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
                ov_pause_active: (ov_pause_secs > 0) || (uv_pause_secs > 0) || (oc_pause_secs > 0),
                imbalance_pause_active,
            };
            sc8815_alerts_publisher.publish_immediate(alerts_payload);
        }

        // 100ms 节拍 → 每 1 秒递减 *_secs 计时器
        if adapter_holdoff_secs > 0 {
            adapter_holdoff_secs = adapter_holdoff_secs.saturating_sub(1);
        }
        tick_100ms = tick_100ms.wrapping_add(1);
        if tick_100ms >= 10 {
            tick_100ms = 0;
            if ov_pause_secs > 0 {
                ov_pause_secs = ov_pause_secs.saturating_sub(1);
            }
            if uv_pause_secs > 0 {
                uv_pause_secs = uv_pause_secs.saturating_sub(1);
            }
            if oc_pause_secs > 0 {
                oc_pause_secs = oc_pause_secs.saturating_sub(1);
            }
        }
        let pause_active =
            (ov_pause_secs > 0) || (uv_pause_secs > 0) || (oc_pause_secs > 0) || temp_pause_cmd;
        let sc_active_flag = _adapter_present && (charger_active || charge_confirmed);
        refresh_state_flags(StateFlagsCtx {
            ac_present: _adapter_present,
            sc_active: sc_active_flag,
            charger_active,
            charge_confirmed,
            pause_active,
            imbalance_pause_active,
            full_latched,
            sc_fault: sc_fault_flag,
        });
        Timer::after(Duration::from_millis(100)).await;
    }
}
