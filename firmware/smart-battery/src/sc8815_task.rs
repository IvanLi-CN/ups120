use bq769x0_async_rs::registers::SysStatFlags;
use defmt::{debug, error, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_stm32::{gpio::Output, i2c::I2c};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use sc8815::{
    AdcMeasurements as ScAdcMeasurements, DeadTime, DeviceConfiguration, IbatRatio, IbusRatio,
    OperatingMode, SC8815, SC8815Status, SwitchingFrequency,
};

use crate::data_types::BalancingCvRequest;
use crate::data_types::{Sc8815Alerts, Sc8815Measurements};
use crate::shared::{
    BalancingCvRequestSubscriber, Bq76920MeasurementsSubscriber, Sc8815AlertsPublisher,
    Sc8815MeasurementsPublisher,
};
use crate::{
    charger_control::{self, ChargeSpeedSetting, limits_for},
    state_bits::{self, bits as sbits, pause_bits},
};
// EXTI is handled by irq_mux; no direct dependency here
use portable_atomic::AtomicBool;

pub const SC8815_DEFAULT_ADDRESS: u8 = sc8815::registers::constants::DEFAULT_ADDRESS;

const RSENSE_MOHM: u16 = 10;
const IBUS_RATIO: IbusRatio = IbusRatio::Ratio3x;
const IBAT_RATIO: IbatRatio = IbatRatio::Ratio12x;

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
const ADAPTER_RELOG_INTERVAL_MS: u32 = 3_000;
const GATE_RELOG_INTERVAL_MS: u32 = 3_000;

macro_rules! sc_diag {
    ($($arg:tt)*) => {
        if ENABLE_SC8815_DIAG {
            defmt::info!($($arg)*);
        }
    };
}

#[inline(always)]
fn log_adapter_event(tag: char, hold_secs: u8, ac_present: bool, manual_override: bool) {
    defmt::debug!(
        "sc:adp e={} hs={} ac={} mo={}",
        tag,
        hold_secs,
        ac_present,
        manual_override
    );
}

#[inline(always)]
fn pack_status_bits(status: &SC8815Status) -> u8 {
    ((status.ac_adapter_connected as u8) << 0)
        | ((status.usb_load_detected as u8) << 1)
        | ((status.eoc as u8) << 2)
        | ((status.otp_fault as u8) << 3)
        | ((status.vbus_short_fault as u8) << 4)
}

// SC8815 ADIN temperature policy constants (see SOFTWARE_DESIGN.md §11)
// Run-mode (power stage enabled): VCC_SC≈5V on this board → 50°C code≈220
const ADIN_CODE_HOT_STOP_5V: u16 = 220; // 50°C @5V
// Stop-mode (power stage stopped): use 3V mapping for resume/cold after settle
const ADIN_CODE_RESUME_3V: u16 = 178; // 40°C @3V
const ADIN_CODE_COLD_3V: u16 = 593; // 0°C  @3V
// Tuning knobs (kept conservative after verification)
const ADIN_CODE_MARGIN: u16 = 2; // ±2 codes tolerance
const ADIN_DEBOUNCE_SAMPLES: u8 = 2; // consecutive samples
// After asserting PSTOP (stop power stage), allow VCC_SC to drop to its low rail
// before evaluating stop-mode (3V) thresholds to avoid immediate RESUME chatter.
const VCCSC_DROP_MS: u32 = 10_000; // settle window; tuned per bench (10 s)
const ENABLE_ADIN_SNAP: bool = false; // disable per-sample log to save flash

// (Note) Removed LUT helpers to minimize footprint; thresholds use precomputed codes.

// Local alias for the concrete I2C device type used by this task.
type I2cDev = I2cDevice<
    'static,
    CriticalSectionRawMutex,
    I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
>;

#[inline(never)]
#[cold]
async fn sc_end_and_dock(
    sc8815_session: &mut Option<ScSession>,
    ce_ctl_slot: &mut Option<Output<'static>>,
    pstop_ctl_slot: &mut Option<Output<'static>>,
    parked_i2c_device: &mut Option<I2cDev>,
) {
    if let Some(sess) = sc8815_session.take() {
        let (ce_back, pstop_back, i2c_back) = sess.end().await;
        *ce_ctl_slot = Some(ce_back);
        *pstop_ctl_slot = Some(pstop_back);
        *parked_i2c_device = Some(i2c_back);
    } else {
        if let Some(pin) = pstop_ctl_slot.as_mut() {
            pin.set_low();
        }
        if let Some(pin) = ce_ctl_slot.as_mut() {
            // Keep CE enabled when docked to allow INT and avoid re-init
            pin.set_high();
        }
    }
}

// Session struct encapsulating an active SC8815 instance and related resources.
struct ScSession {
    sc: SC8815<I2cDev>,
    ce_ctl: Output<'static>,
    pstop_ctl: Output<'static>,
    current_speed: ChargeSpeedSetting,
    ibus_ratio: IbusRatio,
    ibat_ratio: IbatRatio,
    rs1_mohm: u16,
    rs2_mohm: u16,
}

// SC8815 INT → 事件通知（EXTI 边沿触发后唤醒本任务查询状态）
static SC_INT_PENDING: AtomicBool = AtomicBool::new(false);
// Ensure SC8815.init() is executed only once across device lifetime
static SC_INIT_DONE: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn set_sc_int_pending() {
    SC_INT_PENDING.store(true, portable_atomic::Ordering::Relaxed);
    crate::sleep_manager::bump("sc-int");
}

// SC EXTI is handled in irq_mux::irq_mux_task

impl ScSession {
    async fn begin(
        mut ce_pin: Output<'static>,
        pstop_pin: Output<'static>,
        i2c: I2cDev,
        address: u8,
        speed: ChargeSpeedSetting,
    ) -> Result<Self, (Output<'static>, Output<'static>, I2cDev)> {
        // Power-up sequence: CE_CTL=High (chip CE=Enable via inversion)
        ce_pin.set_high();
        Timer::after(Duration::from_millis(100)).await;

        let mut sc = SC8815::new(i2c, address);
        // Initialize only once; subsequent sessions reuse existing configuration
        if !SC_INIT_DONE.load(portable_atomic::Ordering::Relaxed) {
            if let Err(_e) = sc.init().await {
                error!("sc_init_err");
                let i2c_back = sc.release();
                ce_pin.set_high();
                return Err((ce_pin, pstop_pin, i2c_back));
            }
            SC_INIT_DONE.store(true, portable_atomic::Ordering::Relaxed);
        }

        // Per-session configuration
        let mut device_config = DeviceConfiguration::default();
        device_config.battery.use_internal_setting = false; // External divider
        device_config.current_limits.rs1_mohm = RSENSE_MOHM;
        device_config.current_limits.rs2_mohm = RSENSE_MOHM;
        device_config.current_limits.ibus_ratio = IBUS_RATIO;
        device_config.current_limits.ibat_ratio = IBAT_RATIO;
        let limits = limits_for(speed);
        device_config.current_limits.ibus_limit_ma = limits.ibus_limit_ma;
        device_config.current_limits.ibat_limit_ma = limits.ibat_limit_ma;
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
            current_speed: speed,
            ibus_ratio: IBUS_RATIO,
            ibat_ratio: IBAT_RATIO,
            rs1_mohm: RSENSE_MOHM,
            rs2_mohm: RSENSE_MOHM,
        })
    }

    async fn end(mut self) -> (Output<'static>, Output<'static>, I2cDev) {
        if let Err(_e) = self.sc.set_adc_conversion(false).await {
            error!("sc_adc_stop");
        }
        let i2c_back = self.sc.release();
        // Keep power stage stopped, but leave CE enabled to preserve configuration
        self.ce_ctl.set_high();
        self.pstop_ctl.set_low();
        (self.ce_ctl, self.pstop_ctl, i2c_back)
    }

    async fn apply_speed(&mut self, speed: ChargeSpeedSetting) -> Result<(), ()> {
        if self.current_speed == speed {
            return Ok(());
        }
        let limits = limits_for(speed);
        if let Err(_e) = self
            .sc
            .set_ibus_limit(limits.ibus_limit_ma, self.ibus_ratio.into(), self.rs1_mohm)
            .await
        {
            warn!("sc:set_ibus_fail");
            return Err(());
        }
        if let Err(_e) = self
            .sc
            .set_ibat_limit(limits.ibat_limit_ma, self.ibat_ratio.into(), self.rs2_mohm)
            .await
        {
            warn!("sc:set_ibat_fail");
            return Err(());
        }
        self.current_speed = speed;
        Ok(())
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

#[derive(Copy, Clone, PartialEq, Eq)]
struct GateReport {
    auto_enabled: bool,
    manual_register: bool,
    manual_allow: bool,
    manual_override: bool,
    ac_present: bool,
    quiesced: bool,
    pstop_requested: bool,
    session_active: bool,
    hold_active: bool,
    ov_pause: bool,
    uv_pause: bool,
    oc_pause: bool,
    pause_other: bool,
    hold_secs: u8,
    vb_mv: i32,
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
    sc_diag!("pstop_ctl=L (stop) at init");

    // Keep I2C device parked here; move into SC8815 only during an active session.
    let mut parked_i2c_device = Some(i2c_device);
    let mut sc8815_session: Option<ScSession> = None;
    // When no active session, pins stay here.
    let mut ce_ctl_slot: Option<Output<'static>> = Some(ce_ctl);
    let mut pstop_ctl_slot: Option<Output<'static>> = Some(pstop_ctl);

    let mut charger_active = false;
    let mut charger_active_prev = false; // track run→stop edge for settle window
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
    // 来自 SC8815 ADIN 的温度暂停（本任务计算）
    let mut sc_temp_pause_active: bool = false;
    let mut sc_temp_pause_prev: bool = false; // for edge logs
    let mut sc_overtemp_adin: bool = false; // indication-only flag
    let mut last_temp_stop_ms: u32 = 0; // when HOT asserted
    let mut last_temp_stop_cause_hot: bool = false; // distinguish HOT vs COLD pause cause
    let mut cold_latch_active: bool = false; // latch low-temp until >=0°C after settle
    // ADIN VCC selection (3V vs 5V) — board behavior:
    // Power stage running → use 5 V codes; power stage stopped → evaluate with 3 V
    // codes after the settle window. Do not assume pin‑level polarity here.
    // 去抖计数器
    let mut hot_hits: u8 = 0;
    let mut cool_hits: u8 = 0;
    let mut cold_hits: u8 = 0;
    // 100ms tick accumulator → drive *_pause_secs at 1 Hz
    let mut tick_100ms: u8 = 0;
    let mut sc_comm_fail_streak: u8 = 0;
    let mut sc_fault_flag: bool = false;
    let mut last_status_eoc: bool = false;
    let mut full_enter_ms: u32 = 0;
    let mut full_exit_ms: u32 = 0;
    let mut full_latched: bool = false;
    let mut applied_speed = charger_control::current_speed();
    let mut last_status_snapshot: Option<SC8815Status> = None;
    let mut last_adapter_logged: Option<bool> = None;
    let mut last_adapter_log_ms: u32 = 0;
    let mut last_expected_log: Option<bool> = None;
    let mut last_confirmed_log: Option<bool> = None;
    let mut latest_adc_snapshot: Option<ScAdcMeasurements> = None;
    let mut last_gate_report: Option<GateReport> = None;
    let mut last_gate_log_ms: u32 = 0;
    let mut pause_cause_bits: u8 = 0;
    // 会话启动后的“AC确认宽限期”：在该窗口内，忽略 STATUS.ac_adapter_connected=false，
    // 仅拦截硬故障，避免热插瞬间或迟滞导致的误判使会话立即被停止。
    let mut ac_confirm_deadline_ms: u16 = 0;

    // Log de-noising latch for pause state changes only
    let mut last_pause_report: Option<(bool, bool)> = None; // (ov_pause_active, imbalance_pause_active)
    // quiesce INT mode: no probe state needed
    // dropout counters omitted in this step to keep flash within limits
    // Boot probe: unconditionally attempt to initialize SC8815 once at power-up
    if sc8815_session.is_none()
        && let (Some(ce_tmp), Some(pstop_tmp)) = (ce_ctl_slot.take(), pstop_ctl_slot.take())
    {
        let cfg_snapshot = charger_control::snapshot();
        // Ensure power stage is stopped during probe
        let mut pstop_tmp = pstop_tmp;
        pstop_tmp.set_low();
        match ScSession::begin(
            ce_tmp,
            pstop_tmp,
            parked_i2c_device.take().expect("I2C missing"),
            address,
            cfg_snapshot.speed,
        )
        .await
        {
            Ok(session) => {
                sc8815_session = Some(session);
                sc_diag!("sc:probe ok");
                crate::failsafe::set_sc_online(true);
            }
            Err((ce_back, pstop_back, i2c_back)) => {
                ce_ctl_slot = Some(ce_back);
                pstop_ctl_slot = Some(pstop_back);
                parked_i2c_device = Some(i2c_back);
                sc_diag!("sc:probe fail");
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
                    || temp_pause_cmd
                    || sc_temp_pause_active,
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

        // Reset pause cause accumulator each loop
        pause_cause_bits = 0;

        let control_snapshot = charger_control::snapshot();
        // 现场排查：一次性输出当前充电控制寄存器映射
        static CTRL_LOGGED: AtomicBool = AtomicBool::new(false);
        if !CTRL_LOGGED.swap(true, portable_atomic::Ordering::Relaxed) {
            defmt::info!(
                "sc:ctrl auto={} manual={} speed={}",
                control_snapshot.auto_enabled,
                control_snapshot.manual_enable,
                control_snapshot.speed as u8
            );
        }
        let manual_override = charger_control::manual_override_active();
        crate::failsafe::set_manual_override(manual_override);
        if control_snapshot.speed != applied_speed {
            applied_speed = control_snapshot.speed;
            if let Some(sess) = sc8815_session.as_mut() {
                if sess.apply_speed(applied_speed).await.is_err() {
                    warn!("sc:speed_apply_fail");
                }
            }
        }

        if !control_snapshot.auto_enabled && !control_snapshot.manual_enable {
            if sc8815_session.is_some() {
                sc_end_and_dock(
                    &mut sc8815_session,
                    &mut ce_ctl_slot,
                    &mut pstop_ctl_slot,
                    &mut parked_i2c_device,
                )
                .await;
            }
            charger_active = false;
            charge_confirmed = false;
            confirm_streak = 0;
            drop_streak = 0;
        }

        if let Some(bq_meas) = latest_bq_measurements.as_ref() {
            let pack_voltage_mv = bq_meas.core_measurements.total_voltage_mv;
            let system_status_flags = bq_meas.core_measurements.system_status.0;
            let ov_fault = system_status_flags.contains(SysStatFlags::OV);
            let uv_fault = system_status_flags.contains(SysStatFlags::UV);
            let scd_fault = system_status_flags.contains(SysStatFlags::SCD);
            let ocd_fault = system_status_flags.contains(SysStatFlags::OCD);
            let critical_fault = ov_fault || uv_fault || scd_fault || ocd_fault;
            if adapter_holdoff_secs > 0 {
                pause_cause_bits |= pause_bits::HOLD_OFF;
            }

            let mut manual_allow = true;
            if !control_snapshot.auto_enabled {
                manual_allow = control_snapshot.manual_enable;
            }

            if pack_voltage_mv <= PACK_OUTPUT_CUTOFF_THRESHOLD_MV {
                if charger_active {
                    sc_diag!("cutoff {}", pack_voltage_mv);
                }
                sc_end_and_dock(
                    &mut sc8815_session,
                    &mut ce_ctl_slot,
                    &mut pstop_ctl_slot,
                    &mut parked_i2c_device,
                )
                .await;
                charger_active = false;
                charge_confirmed = false;
                confirm_streak = 0;
                drop_streak = 0;
            } else if pack_voltage_mv >= PACK_CHARGE_STOP_THRESHOLD_MV {
                if !latest_bal_req.require_cv {
                    sc_diag!(
                        "stop vb>{} {}",
                        PACK_CHARGE_STOP_THRESHOLD_MV,
                        pack_voltage_mv
                    );
                    sc_end_and_dock(
                        &mut sc8815_session,
                        &mut ce_ctl_slot,
                        &mut pstop_ctl_slot,
                        &mut parked_i2c_device,
                    )
                    .await;
                    charger_active = false;
                    charge_confirmed = false;
                    confirm_streak = 0;
                    drop_streak = 0;
                }
            } else if critical_fault {
                if charger_active {
                    sc_diag!("blocking_fault {}", pack_voltage_mv);
                }
                // 打印阻断充电的故障原因
                sc_diag!(
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
                pause_cause_bits |= pause_bits::OVUV_OC;
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
                        if spread >= 100 {
                            if !imbalance_pause_active {
                                imbalance_pause_active = true;
                                defmt::info!("sc:imbalance pause start spread={}mV", spread);
                            }
                        } else if imbalance_pause_active && spread < 50 {
                            imbalance_pause_active = false;
                            defmt::info!("sc:imbalance cleared spread={}mV", spread);
                        }
                    }
                }

                // If in pause (OV or imbalance) and charger is active, gate power stage
                let imbalance_block = imbalance_pause_active;
                if ((ov_pause_secs > 0 || imbalance_block) || temp_pause_cmd) && charger_active {
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
                let request_active = if control_snapshot.auto_enabled {
                    pack_voltage_mv < PACK_CHARGE_START_THRESHOLD_MV
                } else {
                    manual_allow
                };
                defmt::info!(
                    "sc:req active={} auto={} manual_allow={} vb={}mV ov={} uv={} scd={} ocd={} temp_pause={} adin_pause={} hold={}s",
                    request_active,
                    control_snapshot.auto_enabled,
                    manual_allow,
                    pack_voltage_mv,
                    ov_fault,
                    uv_fault,
                    scd_fault,
                    ocd_fault,
                    temp_pause_cmd,
                    sc_temp_pause_active,
                    adapter_holdoff_secs
                );
                if imbalance_pause_active {
                    pause_cause_bits |= pause_bits::IMBALANCE;
                }
                let pol_start_cond = request_active
                    && !charger_active
                    && adapter_holdoff_secs == 0
                    && !temp_pause_cmd
                    && !sc_temp_pause_active
                    && !imbalance_block;

                if request_active && !charger_active && !pol_start_cond {
                    let gate_sig = GateReport {
                        auto_enabled: control_snapshot.auto_enabled,
                        manual_register: control_snapshot.manual_enable,
                        manual_allow,
                        manual_override,
                        ac_present: _adapter_present,
                        quiesced: crate::failsafe::is_quiesced(),
                        pstop_requested: crate::failsafe::is_pstop_requested(),
                        session_active: sc8815_session.is_some(),
                        hold_active: adapter_holdoff_secs > 0,
                        ov_pause: ov_pause_secs > 0,
                        uv_pause: uv_pause_secs > 0,
                        oc_pause: oc_pause_secs > 0,
                        pause_other: imbalance_block || temp_pause_cmd || sc_temp_pause_active,
                        hold_secs: adapter_holdoff_secs,
                        vb_mv: pack_voltage_mv,
                    };
                    let now_ms_gate = embassy_time::Instant::now().as_millis() as u32;
                    let gate_changed = last_gate_report != Some(gate_sig);
                    let relog_due =
                        now_ms_gate.wrapping_sub(last_gate_log_ms) >= GATE_RELOG_INTERVAL_MS;
                    if gate_changed || relog_due {
                        let blocked_reason = if gate_sig.hold_active {
                            "holdoff"
                        } else if gate_sig.pause_other {
                            "pause"
                        } else if gate_sig.ov_pause || gate_sig.uv_pause || gate_sig.oc_pause {
                            "fault_pause"
                        } else if gate_sig.quiesced || gate_sig.pstop_requested {
                            "failsafe"
                        } else {
                            "unknown"
                        };
                        // 提升到 info，便于现场排查手动模式下为何未启动
                        defmt::info!(
                            "sc:gate block={} auto={} mr={} allow={} ovrd={} ac={} q={} pst={} s={} h={} ov={} uv={} oc={} pa={} vb={}mV",
                            blocked_reason,
                            gate_sig.auto_enabled,
                            gate_sig.manual_register,
                            gate_sig.manual_allow,
                            gate_sig.manual_override,
                            gate_sig.ac_present,
                            gate_sig.quiesced,
                            gate_sig.pstop_requested,
                            gate_sig.session_active,
                            gate_sig.hold_secs,
                            gate_sig.ov_pause,
                            gate_sig.uv_pause,
                            gate_sig.oc_pause,
                            gate_sig.pause_other,
                            gate_sig.vb_mv
                        );
                        last_gate_report = Some(gate_sig);
                        last_gate_log_ms = now_ms_gate;
                    }
                } else if charger_active {
                    last_gate_report = None;
                }

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
                        match ScSession::begin(
                            ce_take,
                            pstop_take,
                            i2c_take,
                            address,
                            control_snapshot.speed,
                        )
                        .await
                        {
                            Ok(session) => {
                                sc8815_session = Some(session);
                                // 启动确认窗口（例如 800ms）
                                ac_confirm_deadline_ms = (embassy_time::Instant::now().as_millis()
                                    as u16)
                                    .wrapping_add(800);
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
                                sc_diag!("sc:backoff 5s");
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
                        && !sc_temp_pause_active
                    {
                        if let Some(sess) = sc8815_session.as_mut() {
                            let _ = sess.apply_speed(control_snapshot.speed).await;
                            sess.enable_power_stage();
                            defmt::info!(
                                "sc:start vb={}mV speed={}",
                                pack_voltage_mv,
                                control_snapshot.speed as u8
                            );
                        }
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
                            imbalance_pause_active || temp_pause_cmd || sc_temp_pause_active,
                        );
                        if last_pause_report != Some(pause_sig) {
                            let mut b: u8 = 0; // bit0=LH, bit1=OV/UV/OC, bit2=IMB, bit3=BQtemp, bit4=ADINtemp
                            b |= 1 << 0; // LH
                            if ov_pause_secs > 0 || uv_pause_secs > 0 || oc_pause_secs > 0 {
                                b |= 1 << 1;
                                pause_cause_bits |= pause_bits::OVUV_OC;
                            }
                            if imbalance_pause_active {
                                b |= 1 << 2;
                                pause_cause_bits |= pause_bits::IMBALANCE;
                            }
                            if temp_pause_cmd {
                                b |= 1 << 3;
                                pause_cause_bits |= pause_bits::PACK_TEMP;
                            }
                            if sc_temp_pause_active {
                                b |= 1 << 4;
                                pause_cause_bits |= pause_bits::CHG_TEMP;
                            }
                            defmt::info!(
                                "sc:pause sig=0b{:05b} ov={} uv={} oc={} imb={} bqtemp={} adintemp={} hold={}s",
                                b,
                                ov_pause_secs,
                                uv_pause_secs,
                                oc_pause_secs,
                                imbalance_pause_active,
                                temp_pause_cmd,
                                sc_temp_pause_active,
                                adapter_holdoff_secs
                            );
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
                Some(sess) => match embassy_time::with_timeout(
                    Duration::from_secs(2),
                    sess.sc.get_device_status(),
                )
                .await
                {
                    Ok(Ok(status)) => {
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

                        let status_bits = pack_status_bits(&status);
                        if last_status_snapshot.map(|prev| pack_status_bits(&prev))
                            != Some(status_bits)
                        {
                            defmt::debug!(
                                "sc8815: status=0x{:02X} ac={} eoc={} otp={} vbus_short={}",
                                status_bits,
                                status.ac_adapter_connected,
                                status.eoc,
                                status.otp_fault,
                                status.vbus_short_fault
                            );
                        }
                        last_status_snapshot = Some(status);

                        // Track adapter presence
                        if !status.ac_adapter_connected {
                            _adapter_present = false;

                            // 对于自动策略保持保守：无适配器时结束会话并进入冷却；
                            // 手动策略（AUTO=0）不把“适配器不存在”视作安全故障，只记录日志。
                            if control_snapshot.auto_enabled
                                && (now_ms as u16) >= ac_confirm_deadline_ms
                            {
                                pause_cause_bits |= pause_bits::ADAPTER_MISS;
                                sc_end_and_dock(
                                    &mut sc8815_session,
                                    &mut ce_ctl_slot,
                                    &mut pstop_ctl_slot,
                                    &mut parked_i2c_device,
                                )
                                .await;
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
                                state_bits::update_pause_cause(pause_cause_bits);
                                _latest_status_for_alerts = Some(status);
                                let relog_due = now_ms.wrapping_sub(last_adapter_log_ms)
                                    >= ADAPTER_RELOG_INTERVAL_MS;
                                if last_adapter_logged != Some(false) || relog_due {
                                    log_adapter_event(
                                        'L',
                                        adapter_holdoff_secs,
                                        _adapter_present,
                                        manual_override,
                                    );
                                    last_adapter_logged = Some(false);
                                    last_adapter_log_ms = now_ms;
                                }
                                // Skip further work in this tick when adapter just lost
                                continue;
                            }

                            // 手动模式：只记日志，不触发停机/冷却。
                            let relog_due = now_ms.wrapping_sub(last_adapter_log_ms)
                                >= ADAPTER_RELOG_INTERVAL_MS;
                            if last_adapter_logged != Some(false) || relog_due {
                                log_adapter_event(
                                    'L',
                                    adapter_holdoff_secs,
                                    _adapter_present,
                                    manual_override,
                                );
                                last_adapter_logged = Some(false);
                                last_adapter_log_ms = now_ms;
                            }
                        } else {
                            _adapter_present = true;
                            // 一旦确认连接，关闭确认窗口
                            ac_confirm_deadline_ms = 0;
                            let relog_due = now_ms.wrapping_sub(last_adapter_log_ms)
                                >= ADAPTER_RELOG_INTERVAL_MS;
                            if last_adapter_logged != Some(true) || relog_due {
                                log_adapter_event(
                                    'P',
                                    adapter_holdoff_secs,
                                    _adapter_present,
                                    manual_override,
                                );
                                last_adapter_logged = Some(true);
                                last_adapter_log_ms = now_ms;
                            }
                        }

                        // EOC: terminate charging session immediately per device indication
                        if status.eoc {
                            sc_diag!("eoc");
                            pause_cause_bits |= pause_bits::EOC_FULL;
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
                            state_bits::update_pause_cause(pause_cause_bits);
                            _latest_status_for_alerts = Some(status);
                            continue;
                        }

                        if status.otp_fault || status.vbus_short_fault {
                            sc_diag!(
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
                    Ok(Err(_)) | Err(_) => {
                        error!("sc:status!");
                        // err
                        sc_end_and_dock(
                            &mut sc8815_session,
                            &mut ce_ctl_slot,
                            &mut pstop_ctl_slot,
                            &mut parked_i2c_device,
                        )
                        .await;
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
                    sc_diag!("sc_session_missing");
                    if let Some(pin) = pstop_ctl_slot.as_mut() {
                        pin.set_low();
                    }
                    if let Some(pin) = ce_ctl_slot.as_mut() {
                        // Leave CE enabled; session is missing but chip stays online
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
                            // snap
                        }
                        let now_ms = embassy_time::Instant::now().as_millis() as u32;
                        crate::failsafe::sc_heartbeat_update(now_ms);
                        crate::failsafe::set_sc_online(true);
                        sc_comm_fail_streak = 0;
                        // ok

                        // --- Temperature policy via ADIN ---
                        let adin_mv = measurements.adin_mv;
                        // code ≈ V/2mV - 1, clamp at 0
                        let adin_code: u16 = adin_mv.saturating_div(2).saturating_sub(1);
                        // 档位选择：功率级运行→5V，停机→3V（窗口后评估）。
                        latest_adc_snapshot = Some(measurements);

                        if charger_active {
                            // Running：按板上实际→ VCC_SC≈5V，采用 5V 映射阈值
                            let hot_code = ADIN_CODE_HOT_STOP_5V;
                            if adin_code <= hot_code.saturating_add(ADIN_CODE_MARGIN) {
                                hot_hits = hot_hits.saturating_add(1);
                            } else {
                                hot_hits = 0;
                            }
                            sc_overtemp_adin = hot_hits >= ADIN_DEBOUNCE_SAMPLES;
                            if sc_overtemp_adin {
                                // Enter temperature pause: stop power stage immediately
                                if let Some(s) = sc8815_session.as_mut() {
                                    s.disable_power_stage();
                                }
                                charger_active = false;
                                charge_confirmed = false;
                                confirm_streak = 0;
                                drop_streak = 0;
                                sc_temp_pause_active = true;
                                last_temp_stop_ms = now_ms;
                                last_temp_stop_cause_hot = true;
                                hot_hits = 0;
                                if !sc_temp_pause_prev {
                                    defmt::warn!("HOT a={} c={}", adin_mv, adin_code);
                                }
                            }
                        } else {
                            // Stopped: wait for VCC_SC to drop before 3V-mapped checks
                            let elapsed_ms = now_ms.saturating_sub(last_temp_stop_ms);
                            if last_temp_stop_ms != 0 && elapsed_ms < VCCSC_DROP_MS {
                                // Hold: do not evaluate resume yet; keep paused
                                cold_hits = 0;
                                cool_hits = 0;
                                sc_temp_pause_active = true;
                                sc_overtemp_adin = false;
                                // fall through to end of branch
                            } else {
                                // Per requirement use 3V mapping for resume/cold
                                sc_overtemp_adin = false; // 指示仅在运行态考虑
                                // Too-cold inhibit (≤0°C): 3V code≥593
                                if adin_code >= ADIN_CODE_COLD_3V.saturating_sub(ADIN_CODE_MARGIN) {
                                    cold_hits = cold_hits.saturating_add(1);
                                } else {
                                    cold_hits = 0;
                                }
                                if cold_hits >= ADIN_DEBOUNCE_SAMPLES {
                                    sc_temp_pause_active = true; // latch pause while too cold
                                    cold_latch_active = true;
                                    last_temp_stop_cause_hot = false;
                                    if !sc_temp_pause_prev {
                                        defmt::warn!("COLD a={} c>={}", adin_mv, ADIN_CODE_COLD_3V);
                                    }
                                }

                                // Resume path depends on cause:
                                if cold_latch_active {
                                    // cold latch: release when >=0°C (3V code <593)
                                    if adin_code
                                        < ADIN_CODE_COLD_3V.saturating_sub(ADIN_CODE_MARGIN)
                                    {
                                        cool_hits = cool_hits.saturating_add(1);
                                    } else {
                                        cool_hits = 0;
                                    }
                                    if cool_hits >= ADIN_DEBOUNCE_SAMPLES {
                                        cold_latch_active = false;
                                        sc_temp_pause_active = false;
                                        cool_hits = 0;
                                        sc_diag!(
                                            "RESUME_COLD a={} c<{}",
                                            adin_mv,
                                            ADIN_CODE_COLD_3V
                                        );
                                    }
                                } else if last_temp_stop_cause_hot {
                                    // hot stop: release when ≤40°C (3V code ≥178)
                                    if adin_code
                                        >= ADIN_CODE_RESUME_3V.saturating_sub(ADIN_CODE_MARGIN)
                                    {
                                        cool_hits = cool_hits.saturating_add(1);
                                    } else {
                                        cool_hits = 0;
                                    }
                                    if cool_hits >= ADIN_DEBOUNCE_SAMPLES {
                                        sc_temp_pause_active = false;
                                        last_temp_stop_cause_hot = false;
                                        cool_hits = 0;
                                        sc_diag!("RESUME a={} c>={}", adin_mv, ADIN_CODE_RESUME_3V);
                                    }
                                } else {
                                    // No ADIN cause remains and settle window is over → resume
                                    if sc_temp_pause_active {
                                        sc_temp_pause_active = false;
                                        sc_diag!(
                                            "RESUME_WIN a={} c={} (no-cause)",
                                            adin_mv,
                                            adin_code
                                        );
                                    }
                                }
                            }
                        }

                        // Track edge
                        sc_temp_pause_prev = sc_temp_pause_active;

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
                                sc_diag!("sc:chg_ok {}mA", ibat);
                            }

                            if charge_confirmed && drop_streak >= CHARGE_CONFIRMATION_SAMPLES {
                                charge_confirmed = false;
                                sc_diag!("sc:chg_lost {}", ibat);
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
                    temp_pause_adin: sc_temp_pause_active,
                    overtemp_adin: sc_overtemp_adin,
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
                temp_pause_adin: sc_temp_pause_active,
                overtemp_adin: sc_overtemp_adin,
            };
            sc8815_alerts_publisher.publish_immediate(alerts_payload);
        }

        let status_bits_snapshot = last_status_snapshot
            .map(|status| pack_status_bits(&status))
            .unwrap_or(0);

        if last_expected_log != Some(charger_active) {
            defmt::debug!(
                "sc8815: cmd={} conf={} ac={} hold={} stat=0x{:02X}",
                charger_active,
                charge_confirmed,
                _adapter_present,
                adapter_holdoff_secs,
                status_bits_snapshot
            );
            last_expected_log = Some(charger_active);
        }

        if last_confirmed_log != Some(charge_confirmed) {
            if let Some(meas) = latest_adc_snapshot {
                defmt::debug!(
                    "sc8815: confirm={} vb={}mV ibus={}mA ibat={}mA",
                    charge_confirmed,
                    meas.vbat_mv,
                    meas.ibus_ma,
                    meas.ibat_ma
                );
            } else {
                defmt::debug!("sc8815: confirm={}", charge_confirmed);
            }
            last_confirmed_log = Some(charge_confirmed);
        }

        // 100ms 节拍 → 每 1 秒递减 *_secs 计时器
        if adapter_holdoff_secs > 0 {
            adapter_holdoff_secs = adapter_holdoff_secs.saturating_sub(1);
            if adapter_holdoff_secs == 0 {
                log_adapter_event('R', adapter_holdoff_secs, _adapter_present, manual_override);
                if !_adapter_present {
                    log_adapter_event('M', adapter_holdoff_secs, _adapter_present, manual_override);
                }
            }
        }
        let now_ms_loop = embassy_time::Instant::now().as_millis() as u32;
        if !_adapter_present
            && now_ms_loop.wrapping_sub(last_adapter_log_ms) >= ADAPTER_RELOG_INTERVAL_MS
        {
            log_adapter_event('M', adapter_holdoff_secs, _adapter_present, manual_override);
            last_adapter_logged = Some(false);
            last_adapter_log_ms = now_ms_loop;
        }
        tick_100ms = tick_100ms.wrapping_add(1);
        let mut snapshot_due = false;
        if tick_100ms >= 10 {
            tick_100ms = 0;
            snapshot_due = true;
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
        if snapshot_due && (_adapter_present || charger_active || charge_confirmed) {
            if let Some(meas) = latest_adc_snapshot {
                defmt::debug!(
                    "sc8815: snap vb={}mV vbus={}mV ibus={}mA ibat={}mA status=0x{:02X}",
                    meas.vbat_mv,
                    meas.vbus_mv,
                    meas.ibus_ma,
                    meas.ibat_ma,
                    status_bits_snapshot
                );
            }
        }
        // One-shot 10 s settle window after any run→stop edge (no stacking)
        if charger_active_prev && !charger_active {
            let now_ms_edge = embassy_time::Instant::now().as_millis() as u32;
            // Only start window if a previous window is not active
            if last_temp_stop_ms == 0
                || now_ms_edge.saturating_sub(last_temp_stop_ms) >= VCCSC_DROP_MS
            {
                last_temp_stop_ms = now_ms_edge;
                sc_diag!("temp: WIN_START edge");
            }
        }
        charger_active_prev = charger_active;

        let pause_active = (ov_pause_secs > 0)
            || (uv_pause_secs > 0)
            || (oc_pause_secs > 0)
            || temp_pause_cmd
            || sc_temp_pause_active;
        let sc_active_flag = _adapter_present && (charger_active || charge_confirmed);
        charger_control::update_observed_enable(charger_active);
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
        state_bits::update_pause_cause(pause_cause_bits);
        Timer::after(Duration::from_millis(100)).await;
    }
}
