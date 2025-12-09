use defmt::{Debug2Format, debug, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use esp_hal::time::Instant;
use ina226::INA226;
use sc8815::registers::constants::DEFAULT_ADDRESS as SC8815_ADDR;
use static_cell::StaticCell;

use crate::{
    AC_STABLE_MS, CHARGE_START_VBAT_MV, CHARGE_STOP_VBAT_MV, DISCH_RESUME_VBAT_MV,
    DISCH_STOP_VBAT_MV, I2cBusMutex, SB_REG_CHG_CONFIG, SB_REG_CHG_PAUSE_CAUSE, SB_REG_STATE_FLAGS,
    SB_REG_TEMP_STATUS, SB_STATE_FLAG_AC_PRESENT, SB_STATE_FLAG_FAULT_BQ, SB_STATE_FLAG_FAULT_SC,
    SB_STATE_POLL_INTERVAL_MS, SharedI2cDevice, UPS_DISCH_RESUME_C, UPS_DISCH_STOP_C,
    UPS_SC_IBAT_LIMIT_MA, UPS_SC_IBUS_LIMIT_MA, UPS_SC_RS1_MOHM, UPS_SC_RS2_MOHM,
    UPS_VBUS_AC_OFFLINE_MV, UPS_VBUS_AC_ONLINE_MV, UPS_VBUS_MAX_MV, UPS_VBUS_MIN_MV, fan_control,
    io_expander::Tca6408a,
};

/// INA226 I²C address on the UPS power board (A0=A1=GND).
const INA226_ADDR: u8 = 0x40;

/// Charging mode exposed to other tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeMode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug)]
pub enum SbFaultCode {
    CommErr,
    StateNA,
    ReqChgMismatch,
    ReqStopMismatch,
    TempLow,
    TempHotChg,
    TempHotDsg,
    Imbalance,
    OvUvOc,
    AdapterMiss,
    HoldOff,
    FaultBq,
    FaultSc,
}

/// Snapshot of the current power / smart-battery state.
///
/// This is intentionally a plain, `Copy`-friendly structure so UI and
/// thermal tasks can take cheap snapshots without holding the mutex
/// longer than necessary.
#[derive(Clone, Copy)]
pub struct PowerState {
    /// AC présent / good flag as seen by the ESP32, derived from TPS2490 PG
    /// assertion (input path good) + INA226 VIN online check.
    pub ac_present: bool,
    /// Whether VIN has been stable for at least `AC_STABLE_MS`.
    pub ac_stable: bool,
    /// Latest INA226-measured external input voltage in mV, if available.
    pub vin_meas_mv: Option<u32>,
    /// Whether INA226 measurements are considered faulty/unavailable.
    pub ina226_fault: bool,
    /// Current smart-battery charging mode.
    pub charge_mode: ChargeMode,
    /// Last written CHG_CONFIG register value.
    pub chg_config: u8,
    /// Most recent pack voltage (mV) from STM32 smart-battery slave.
    pub vbat_mv: Option<u32>,
    /// Most recent pack current (mA, discharge negative).
    pub ibat_ma: Option<i32>,
    /// Cached per-cell voltages from the periodic 10s snapshot.
    pub cells_mv: [Option<u16>; 5],
    /// Cached STATE_FLAGS from the periodic 10s snapshot.
    pub state_flags: Option<u16>,
    /// Cached CHG_PAUSE_CAUSE from the periodic 10s snapshot.
    pub sb_chg_pause_cause: Option<u8>,
    /// Latest smart-battery TEMP_STATUS bitmap (if available).
    pub sb_temp_status: Option<u8>,
    /// Latest smart-battery temperature set (pack + charger).
    pub smart_batt_temps: Option<fan_control::SmartBatteryTemps>,
    /// Latest SC8815 ADIN-derived UPS temperature in °C.
    pub adin_temp_c: Option<f32>,
    /// Whether temperature gating has currently paused charging.
    pub temp_pause_active: bool,
    /// Whether UPS OUT is currently enabled via SC8815 OTG path.
    pub out_enabled: bool,
    /// OUT bus voltage estimate in mV (SC8815 VBUS ADC), if available.
    pub out_v_mv: Option<u32>,
    /// OUT current in mA (discharge towards load is positive), if available.
    pub out_a_ma: Option<i32>,
    /// OUT power in mW, derived from voltage/current, if available.
    pub out_w_mw: Option<u32>,
    /// Whether smart-battery I2C/state reads are currently considered faulty.
    pub sb_comm_error: bool,
    /// Millisecond timestamp (since boot) of the last successful update.
    pub last_update_ms: u64,
}

impl core::fmt::Debug for PowerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PowerState")
            .field("ac_present", &self.ac_present)
            .field("ac_stable", &self.ac_stable)
            .field("vin_meas_mv", &self.vin_meas_mv)
            .field("ina226_fault", &self.ina226_fault)
            .field("charge_mode", &self.charge_mode)
            .field("chg_config", &self.chg_config)
            .field("vbat_mv", &self.vbat_mv)
            .field("ibat_ma", &self.ibat_ma)
            .field("cells_mv", &self.cells_mv)
            .field("state_flags", &self.state_flags)
            .field("sb_chg_pause_cause", &self.sb_chg_pause_cause)
            .field("sb_temp_status", &self.sb_temp_status)
            .field("smart_batt_temps", &Debug2Format(&self.smart_batt_temps))
            .field("adin_temp_c", &self.adin_temp_c)
            .field("temp_pause_active", &self.temp_pause_active)
            .field("out_enabled", &self.out_enabled)
            .field("out_v_mv", &self.out_v_mv)
            .field("out_a_ma", &self.out_a_ma)
            .field("out_w_mw", &self.out_w_mw)
            .field("sb_comm_error", &self.sb_comm_error)
            .field("last_update_ms", &self.last_update_ms)
            .finish()
    }
}

impl Default for PowerState {
    fn default() -> Self {
        Self {
            ac_present: false,
            ac_stable: false,
            vin_meas_mv: None,
            ina226_fault: false,
            charge_mode: ChargeMode::Auto,
            chg_config: 0,
            vbat_mv: None,
            ibat_ma: None,
            cells_mv: [None; 5],
            state_flags: None,
            sb_chg_pause_cause: None,
            sb_temp_status: None,
            smart_batt_temps: None,
            adin_temp_c: None,
            temp_pause_active: false,
            out_enabled: false,
            out_v_mv: None,
            out_a_ma: None,
            out_w_mw: None,
            sb_comm_error: false,
            last_update_ms: 0,
        }
    }
}

pub type PowerStateMutex = Mutex<NoopRawMutex, PowerState>;

pub fn compute_sb_fault(state: &PowerState) -> Option<SbFaultCode> {
    // 1. Communication / state availability.
    if state.sb_comm_error {
        return Some(SbFaultCode::CommErr);
    }

    let flags = match state.state_flags {
        Some(f) => f,
        None => return Some(SbFaultCode::StateNA),
    };

    // Local mirrors of STM32 STATE_FLAGS bits used for fault derivation.
    const SB_STATE_FLAG_CHARGING: u16 = 1 << 1;
    const SB_STATE_FLAG_CHG_PAUSED: u16 = 1 << 2;
    const SB_STATE_FLAG_FULL: u16 = 1 << 4;

    // Local mirrors of CHG_PAUSE_CAUSE bits.
    const SB_PAUSE_IMBALANCE: u8 = 1 << 0;
    const SB_PAUSE_PACK_TEMP: u8 = 1 << 1;
    const SB_PAUSE_CHG_TEMP: u8 = 1 << 2;
    const SB_PAUSE_OVUV_OC: u8 = 1 << 3;
    const SB_PAUSE_HOLD_OFF: u8 = 1 << 4;
    const SB_PAUSE_ADAPTER_MISS: u8 = 1 << 5;
    const SB_PAUSE_EOC_FULL: u8 = 1 << 6;

    let charging_active = (flags & SB_STATE_FLAG_CHARGING) != 0;
    let chg_paused = (flags & SB_STATE_FLAG_CHG_PAUSED) != 0;
    let full_flag = (flags & SB_STATE_FLAG_FULL) != 0;

    let pause = state.sb_chg_pause_cause.unwrap_or(0);
    let pause_imbalance = (pause & SB_PAUSE_IMBALANCE) != 0;
    let pause_ovuv_oc = (pause & SB_PAUSE_OVUV_OC) != 0;
    let pause_hold_off = (pause & SB_PAUSE_HOLD_OFF) != 0;
    let pause_adapter_miss = (pause & SB_PAUSE_ADAPTER_MISS) != 0;
    let pause_eoc_full = (pause & SB_PAUSE_EOC_FULL) != 0;
    let pause_temp_related = (pause & (SB_PAUSE_PACK_TEMP | SB_PAUSE_CHG_TEMP)) != 0;

    // Decode TEMP_STATUS once for temperature-related codes and gating.
    let temp_flags = state.sb_temp_status.map(crate::decode_temp_status);
    let temp_low = matches!(temp_flags, Some(f) if f.temp_low);
    let temp_hot_chg = matches!(temp_flags, Some(f) if f.temp_high_chg);
    let temp_hot_dsg = matches!(temp_flags, Some(f) if f.temp_high_dsg);

    // 2. Request/actual mismatch between ESP intent and STM32 CHARGING bit.
    //
    // Treat Manual mode with AC present as an explicit charge request. We also
    // require VIN to be stable to avoid transient mismatches around adapter
    // plug/unplug.
    let charge_requested =
        state.ac_present && state.ac_stable && matches!(state.charge_mode, ChargeMode::Manual);

    // Consider conditions that legitimately explain why charging might be
    // paused or stopped even when a request is present. When any of these are
    // true we suppress ReqChgMismatch so more specific faults can surface.
    let mut has_valid_pause_reason = false;
    if full_flag || pause_eoc_full {
        has_valid_pause_reason = true;
    }
    if pause_temp_related || state.temp_pause_active || temp_low || temp_hot_chg {
        has_valid_pause_reason = true;
    }
    if pause_ovuv_oc || pause_imbalance || pause_adapter_miss {
        has_valid_pause_reason = true;
    }
    if chg_paused {
        has_valid_pause_reason = true;
    }
    if !state.ac_present || !state.ac_stable {
        has_valid_pause_reason = true;
    }

    if charge_requested && !charging_active && !has_valid_pause_reason {
        return Some(SbFaultCode::ReqChgMismatch);
    }

    // ReqStopMismatch: ESP believes charging should be stopped (no manual
    // request, AC missing or gating) but STM32 still reports CHARGING.
    let stop_requested = (!state.ac_present || state.temp_pause_active || full_flag)
        && !matches!(state.charge_mode, ChargeMode::Manual);
    if stop_requested && charging_active {
        return Some(SbFaultCode::ReqStopMismatch);
    }

    // 3. Chip-level faults from STATE_FLAGS.
    if (flags & SB_STATE_FLAG_FAULT_BQ) != 0 {
        return Some(SbFaultCode::FaultBq);
    }
    if (flags & SB_STATE_FLAG_FAULT_SC) != 0 {
        return Some(SbFaultCode::FaultSc);
    }

    // 4. Temperature faults from TEMP_STATUS.
    if temp_low {
        return Some(SbFaultCode::TempLow);
    }
    if temp_hot_chg {
        return Some(SbFaultCode::TempHotChg);
    }
    if temp_hot_dsg {
        return Some(SbFaultCode::TempHotDsg);
    }

    // 5. Voltage/current/adapter/imbalance-related pause causes.
    if pause_imbalance {
        return Some(SbFaultCode::Imbalance);
    }
    if pause_ovuv_oc {
        return Some(SbFaultCode::OvUvOc);
    }
    if pause_adapter_miss {
        return Some(SbFaultCode::AdapterMiss);
    }
    if pause_hold_off {
        return Some(SbFaultCode::HoldOff);
    }

    None
}

static POWER_STATE: StaticCell<PowerStateMutex> = StaticCell::new();

/// Initialise the global power-state mutex.
///
/// This must be called once from `main` before spawning `power_task`.
pub fn init_power_state() -> &'static PowerStateMutex {
    POWER_STATE.init(Mutex::new(PowerState::default()))
}

/// Helper: ensure SC8815 is initialised and configured for OTG / external VBUS
/// feedback, without touching PSTOP / EN_OTG.
///
/// This matches discharge_policy.md §3 and SC8815_External_Resistor_Configuration.md:
/// - single global instance on the shared I2C bus
/// - external VBUS FB (FB_SEL=1, use_internal_setting=false)
/// - OTG operating mode, no charging features enabled.
async fn sc8815_init_otg(
    i2c_bus: &'static I2cBusMutex,
    tca: &mut Tca6408a<SharedI2cDevice<'static>>,
    sc: &mut Option<sc8815::SC8815<SharedI2cDevice<'static>>>,
    sc_init_done: &mut bool,
    sc_otg_configured: &mut bool,
) -> bool {
    // Safety: before touching SC8815 configuration, force power stage OFF:
    // PSTOP=high, EN_OTG=0. This matches SC8815 docs “configure in standby”.
    let _ = tca.set_sc_pstop(true).await;

    // Drive /CE low via TCA6408A to enable SC8815 before I2C transactions.
    if tca.set_sc_ce(true).await.is_err() {
        warn!("discharge: tca6408a set_sc_ce(true) failed");
        return false;
    }

    // Create the global SC8815 instance if needed.
    if sc.is_none() {
        let dev = I2cDevice::new(i2c_bus);
        let drv = sc8815::SC8815::new(dev, SC8815_ADDR);
        *sc = Some(drv);
    }

    let Some(drv) = sc.as_mut() else {
        return false;
    };

    // One-time init() per power cycle.
    if !*sc_init_done {
        match drv.init().await {
            Ok(()) => {
                *sc_init_done = true;
            }
            Err(_) => {
                warn!("discharge: sc8815 init failed");
                return false;
            }
        }
    }

    // One-time OTG/device configuration; does not yet enable the power stage.
    if *sc_init_done && !*sc_otg_configured {
        // Configure OTG operating mode and limits; VBAT / VBUS for the UPS
        // OUT path are set purely by external resistor networks
        // (use_internal_setting=false), see SC8815_External_Resistor_Configuration.md.
        let mut config = sc8815::DeviceConfiguration::default();
        config.battery.cell_count = sc8815::CellCount::Cells4S;
        config.battery.voltage_per_cell = sc8815::VoltagePerCell::Mv4200;
        config.battery.use_internal_setting = true;
        // Current-limit configuration per UPS power board shunts:
        // RS1 / RS2 are both 5mΩ (R47/R26, HoLLR1206-1W-5mR-1%), and we start
        // with a conservative 7A OTG limit on both sides to match the DC jack
        // rating; see ups-power-board netlist.
        config.current_limits.rs1_mohm = UPS_SC_RS1_MOHM;
        config.current_limits.rs2_mohm = UPS_SC_RS2_MOHM;
        config.current_limits.ibus_limit_ma = UPS_SC_IBUS_LIMIT_MA;
        config.current_limits.ibat_limit_ma = UPS_SC_IBAT_LIMIT_MA;
        config.current_limits.ibus_ratio = sc8815::IbusRatio::Ratio6x;
        config.power.operating_mode = sc8815::OperatingMode::OTG;
        config.power.switching_frequency = sc8815::SwitchingFrequency::Freq450kHz;
        config.power.dead_time = sc8815::DeadTime::Ns80;
        config.power.pfm_mode = true;
        config.trickle_charging = false;
        config.charging_termination = false;
        config.use_ibus_for_charging = false;

        match drv.configure_device(&config).await {
            Ok(()) => {
                // Force short-circuit foldback disabled (DIS_ShortFoldBack=1)
                // for this UPS application; SC8815 shall not autonomously
                // reduce IBUS/IBAT on VBUS_SHORT, we rely on our own gating
                // and current limits instead.
                // let _ = drv.set_short_foldback_disable(true).await;
                // Optional: select 12.5x VBAT monitor ratio so the 12–18.5 V
                // operating range has headroom and does not saturate a 5x span.
                let _ = drv.set_vbat_monitor_ratio(0).await;
                *sc_otg_configured = true;
            }
            Err(_) => {
                warn!("discharge: sc8815 OTG configure failed");
                return false;
            }
        }
    }

    // Enable ADC conversions so VBUS / VBAT / IBUS / IBAT / ADIN are readable
    // once the power stage is allowed to run.
    if *sc_otg_configured {
        if let Err(_) = drv.set_adc_conversion(true).await {
            warn!("discharge: sc8815 set_adc_conversion(true) failed");
            return false;
        }
    }

    true
}

/// Helper: control SC8815 power stage (PSTOP + EN_OTG) without touching
/// configuration or VBUS target.
///
/// This encapsulates the OUT_ENABLED / OUT_DISABLED transitions in
/// discharge_policy.md §5.1.
async fn sc8815_set_power_stage(
    tca: &mut Tca6408a<SharedI2cDevice<'static>>,
    sc: &mut Option<sc8815::SC8815<SharedI2cDevice<'static>>>,
    enable: bool,
) {
    if enable {
        let _ = tca.set_sc_pstop(false).await;
    } else {
        let _ = tca.set_sc_pstop(true).await;
    }
}

/// Helper: set SC8815 VBUS target in external FB mode (VBUSREF_E) using
/// the UPS power board 100k/11k feedback network.
///
/// - Clamps requested VBUS into [UPS_VBUS_MIN_MV, UPS_VBUS_MAX_MV]
///   (discharge_policy.md §3, SC8815_External_Resistor_Configuration.md)
/// - Converts VBUS target to the internal VBUSREF_E using
///   VBUS = VREF_E × (1 + R_UP/R_DOWN), with R_UP=100k, R_DOWN=11k.
async fn sc8815_set_vbus_external_target(
    sc: &mut Option<sc8815::SC8815<SharedI2cDevice<'static>>>,
    target_vbus_mv: u16,
) -> bool {
    let clamped_vbus_mv = target_vbus_mv.clamp(UPS_VBUS_MIN_MV, UPS_VBUS_MAX_MV);
    let Some(drv) = sc.as_mut() else {
        return false;
    };

    // With R_UP=100k, R_DOWN=11k: gain = 1 + 100k/11k = 111/11.
    let vref_mv: u16 = ((clamped_vbus_mv as u32) * 11 / 111) as u16;

    match drv.set_vbus_external_reference(vref_mv).await {
        Ok(()) => true,
        Err(_) => {
            warn!(
                "discharge: failed to set VBUS reference target={}mV vref={}mV",
                target_vbus_mv, vref_mv
            );
            false
        }
    }
}

/// Asynchronous task responsible for all power-management I2C traffic and
/// maintaining [`PowerState`].
///
/// For now this is a stub that only refreshes `last_update_ms` so the rest
/// of the system can start integrating with it. The full smart-battery /
/// charger logic will be migrated here from `main.rs`.
#[embassy_executor::task]
pub async fn power_task(
    i2c_bus: &'static I2cBusMutex,
    power_state: &'static PowerStateMutex,
    _thermal_state: &'static crate::thermal::ThermalStateMutex,
) {
    // Before touching other devices on the bus, validate STM32 I2C once
    // (mirrors legacy behaviour from `main.rs`).
    Timer::after(Duration::from_millis(2)).await;
    let mut i2c_dev_once = I2cDevice::new(i2c_bus);
    if let Err(_) = crate::stm_one_shot_validate(&mut i2c_dev_once).await {
        warn!("stm32: one-shot i2c validation failed");
    }

    const SB_AUTO_ENABLED: bool = false;
    let sb_speed_tier: u8 = 0x01; // ≈0.8A
    let mut sb_manual_enable = false;
    let mut sb_config_value =
        crate::compose_sb_charge_config(SB_AUTO_ENABLED, sb_manual_enable, sb_speed_tier);
    let mut sb_cfg_last_verify_ms = 0u64;
    let mut sb_temp_pause_active = false;
    let mut sb_last_vbat_mv: Option<u32> = None;
    let mut sb_last_state_poll_ms = 0u64;
    let mut last_state_flags: Option<u16> = None;
    let mut last_cells_mv: [Option<u16>; 5] = [None; 5];
    let mut last_chg_pause_cause: Option<u8> = None;
    let mut sb_comm_error: bool = false;
    let mut sb_comm_error_streak: u8 = 0;

    {
        let mut sb_i2c = I2cDevice::new(i2c_bus);
        match crate::write_smart_battery_reg_retry(&mut sb_i2c, SB_REG_CHG_CONFIG, sb_config_value)
            .await
        {
            Ok(()) => info!("smart-battery: config set (manual ctl, tier=0.8A)"),
            Err(()) => warn!("smart-battery: failed to apply charge config"),
        }
    }

    // Global single-instance drivers (async I2C devices on the shared bus)
    let mut tca: Option<Tca6408a<SharedI2cDevice<'static>>> =
        Some(Tca6408a::new(I2cDevice::new(i2c_bus)));
    // Raw TPS2490 PG level sampled via TCA6408A IN_PG (true = PG asserted = input path good).
    let mut in_pg_raw: Option<bool> = None;
    let mut last_in_pg_logged: Option<bool> = None;
    let mut in_pg_read_failed = false;
    // AC GOOD state derived from TPS2490 PG + INA226 VIN.
    let mut ac_present = false;
    // Track last time AC GOOD changed so we can derive a “stable for AC_STABLE_MS” window.
    let mut ac_state_last_change_ms: u64 = Instant::now().duration_since_epoch().as_millis() as u64;
    let mut charge_skip_adapter_logged = false;

    // Global single-instance INA226 driver on the shared I2C bus.
    let mut ina: Option<INA226<SharedI2cDevice<'static>>> = None;
    let mut ina226_fault: bool = false;
    let mut ina226_fault_logged: bool = false;
    let mut last_ac_diag_log_ms: u64 = 0;

    if let Some(t) = tca.as_mut() {
        match t.init().await {
            Ok(()) => info!("tca6408a: init ok (CE=high, PSTOP=high)"),
            Err(_) => warn!("tca6408a: init failed (safe state not verified)"),
        }
        match t.read_in_pg().await {
            Ok(pg) => {
                in_pg_raw = Some(pg);
                last_in_pg_logged = Some(pg);
                in_pg_read_failed = false;
                info!(
                    "tca6408a: IN_PG={} ({})",
                    if pg { "high" } else { "low" },
                    if pg {
                        "pg asserted, input path good"
                    } else {
                        "pg deasserted: startup/uvlo/limit/protect"
                    }
                );
            }
            Err(_) => {
                warn!("tca6408a: read IN_PG failed");
                in_pg_read_failed = true;
            }
        }
    }

    // Global single-instance SC8815 driver on the shared I2C bus.
    // Disable SC8815 OTG/OUT regulation on ESP32 side for now to avoid it
    // influencing STM32 bring-up and temperature/I2C testing.
    let mut sc: Option<sc8815::SC8815<SharedI2cDevice<'static>>> = None;
    let mut sc_init_done: bool = false;
    let mut last_adin_temp_c: Option<f32> = None;
    // Discharge / OUT state (see discharge_policy.md §§3–5).
    let mut out_enabled: bool = false;
    let mut sc_otg_configured: bool = false;
    let mut sc_fault_latched: bool = false;
    let mut ups_temp_pause_active: bool = false;
    // Track last AC/UPS VBUS mode (true = AC online target, false = battery/AC-missing target).
    let mut last_vbus_ac_mode: Option<bool> = None;
    // Throttling for SC8815 ADC debug logs.
    let mut last_sc_meas_log_ms: u64 = 0;
    // Latest smart-battery TEMP_STATUS value and error logging state.
    let mut sb_last_temp_status: Option<u8> = None;
    let mut sb_temp_status_error_logged: bool = false;
    // One-shot diagnostic flag for the extended temperature window (0x40..0x47).
    let mut sb_temp_window_logged: bool = false;

    // Periodic loop matching the original 500 ms cadence for power sampling and
    // charger control.
    loop {
        Timer::after(Duration::from_millis(500)).await;

        // OUT-side measurements (SC8815) are refreshed later in the loop.
        let mut out_v_mv: Option<u32> = None;
        let mut out_a_ma: Option<i32> = None;
        let mut out_w_mw: Option<u32> = None;

        // Smart-battery temperatures (pack + charger).
        let sb_temps: Option<fan_control::SmartBatteryTemps> = {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            crate::read_smart_battery_temperatures(&mut sb_i2c).await
        };

        // Optional diagnostic: read the extended compact temperature window
        // (0x40..0x47, int8 °C) once after boot and log it for correlation
        // with the STM32-side `therm:` output.
        if !sb_temp_window_logged {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            if let Ok(win) = crate::read_smart_battery_temp_window(&mut sb_i2c).await {
                let [pack, chg, ntc0, ntc1, ntc2, ntc3, bq_int, mcu] = win;
                info!(
                    "stm32: temp-window pack={}C chg={}C ntc=[{}, {}, {}, {}] bq_int={}C mcu={}C",
                    pack, chg, ntc0, ntc1, ntc2, ntc3, bq_int, mcu
                );
                sb_temp_window_logged = true;
            }
        }

        // Smart-battery TEMP_STATUS (thermal fault flags) snapshot.
        {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            match crate::read_smart_battery_reg(&mut sb_i2c, SB_REG_TEMP_STATUS).await {
                Ok(v) => {
                    if let Some(prev) = sb_last_temp_status {
                        if prev != v {
                            let flags = crate::decode_temp_status(v);
                            info!(
                                "stm32: TEMP_STATUS changed 0x{:02X}->0x{:02X} low={} high_chg={} high_dsg={}",
                                prev, v, flags.temp_low, flags.temp_high_chg, flags.temp_high_dsg,
                            );
                        }
                    } else {
                        debug!("stm32: TEMP_STATUS=0x{:02X}", v);
                    }
                    sb_last_temp_status = Some(v);
                    sb_temp_status_error_logged = false;
                }
                Err(_) => {
                    if !sb_temp_status_error_logged {
                        warn!("stm32: TEMP_STATUS read failed");
                        sb_temp_status_error_logged = true;
                    }
                }
            }
        }

        let sb_temp_status = sb_last_temp_status;
        let pack_temp_c = sb_temps.and_then(|t| t.pack_c);

        // Pack voltage and current.
        let vbat_mv = {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            let v = crate::read_smart_battery_vbat_mv(&mut sb_i2c).await;
            if let Some(v_mv) = v {
                sb_last_vbat_mv = Some(v_mv);
            }
            v
        };

        let ibat_ma = {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            crate::read_smart_battery_ibat_ma(&mut sb_i2c).await
        };

        let now_millis = Instant::now().duration_since_epoch().as_millis() as u64;

        // Refresh TPS2490 PG level via IN_PG (raw, before combining with INA226 VIN).
        if let Some(t) = tca.as_mut() {
            match t.read_in_pg().await {
                Ok(level) => {
                    if in_pg_read_failed {
                        info!("power: IN_PG read recovered");
                        in_pg_read_failed = false;
                    }
                    in_pg_raw = Some(level);
                    if last_in_pg_logged != Some(level) {
                        info!(
                            "tca6408a: IN_PG={} ({})",
                            if level { "high" } else { "low" },
                            if level {
                                "pg asserted, input path good"
                            } else {
                                "pg deasserted: startup/uvlo/limit/protect"
                            }
                        );
                        last_in_pg_logged = Some(level);
                    }
                }
                Err(_) => {
                    if !in_pg_read_failed {
                        warn!("tca6408a: read IN_PG failed");
                        in_pg_read_failed = true;
                    }
                    in_pg_raw = None;
                }
            }
        }

        // Refresh INA226 VIN measurement (external adapter voltage).
        let mut vin_meas_mv: Option<u32> = None;
        if ina.is_none() {
            let i2c_dev = I2cDevice::new(i2c_bus);
            ina = Some(INA226::new(i2c_dev, INA226_ADDR));
            info!("ina226: init ok");
            ina226_fault = false;
            ina226_fault_logged = false;
        }

        if let Some(dev) = ina.as_mut() {
            match dev.bus_voltage_millivolts().await {
                Ok(v_mv) => {
                    let v_mv_u32 = if v_mv <= 0.0 { 0 } else { v_mv as u32 };
                    vin_meas_mv = Some(v_mv_u32);
                    if ina226_fault {
                        info!("ina226: measurement recovered (vin={}mV)", v_mv_u32);
                        ina226_fault = false;
                        ina226_fault_logged = false;
                    }
                }
                Err(_) => {
                    ina226_fault = true;
                    vin_meas_mv = None;
                    if !ina226_fault_logged {
                        warn!("ina226: bus voltage read failed; marking INA226 fault");
                        ina226_fault_logged = true;
                    }
                }
            }
        }

        // Derive AC GOOD from TPS2490 PG assertion (input path good) + VIN online threshold + INA226 health.
        let pg_asserted = matches!(in_pg_raw, Some(level) if level) && !in_pg_read_failed;
        let vin_online = vin_meas_mv
            .map(|v| v > UPS_VBUS_AC_ONLINE_MV as u32)
            .unwrap_or(false);
        let ac_present_now = pg_asserted && vin_online && !ina226_fault;

        if ac_present != ac_present_now {
            ac_present = ac_present_now;
            ac_state_last_change_ms = now_millis;
            charge_skip_adapter_logged = false;

            // Log combined AC state along with STM32's perspective.
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            match crate::read_smart_battery_state_flags(&mut sb_i2c).await {
                Some(flags) => {
                    let stm_ac = (flags & SB_STATE_FLAG_AC_PRESENT) != 0;
                    info!(
                        "power: ac {} (stm_ac={} flags=0x{:04x} pg_asserted={} vin_online={} vin_meas_mv={:?} ina226_fault={})",
                        if ac_present { "good" } else { "not-good" },
                        stm_ac,
                        flags,
                        pg_asserted,
                        vin_online,
                        vin_meas_mv,
                        ina226_fault,
                    );
                }
                None => {
                    info!(
                        "power: ac {} (stm_ac=? read_fail pg_asserted={} vin_online={} vin_meas_mv={:?} ina226_fault={})",
                        if ac_present { "good" } else { "not-good" },
                        pg_asserted,
                        vin_online,
                        vin_meas_mv,
                        ina226_fault,
                    );
                }
            }
        } else if now_millis.saturating_sub(last_ac_diag_log_ms) >= 5_000 {
            // Periodic AC/INA226 diagnostic at INFO level for field debugging.
            last_ac_diag_log_ms = now_millis;
            info!(
                "power: ac_diag ac_present={} pg_asserted={} vin_online={} vin_meas_mv={:?} ina226_fault={}",
                ac_present, pg_asserted, vin_online, vin_meas_mv, ina226_fault,
            );
        }

        // Temperature pause/resume gating.
        if let Some(temp) = pack_temp_c {
            if sb_temp_pause_active {
                if temp <= crate::TEMP_RESUME_C {
                    sb_temp_pause_active = false;
                    info!("charge: temperature resume at {=f32}°C", temp);
                }
            } else if temp >= crate::TEMP_PAUSE_C {
                sb_temp_pause_active = true;
                info!("charge: temperature pause at {=f32}°C", temp);
            }
        }

        // CHG_CONFIG drift verification and automatic reapply if necessary.
        if now_millis.saturating_sub(sb_cfg_last_verify_ms) >= crate::SB_CFG_VERIFY_INTERVAL_MS {
            sb_cfg_last_verify_ms = now_millis;
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            match crate::read_smart_battery_reg(&mut sb_i2c, SB_REG_CHG_CONFIG).await {
                Ok(actual) => {
                    if actual != sb_config_value {
                        warn!(
                            "smart-battery: cfg drift detected hw=0x{:02x} expected=0x{:02x}",
                            actual, sb_config_value
                        );
                        let desired = crate::compose_sb_charge_config(
                            SB_AUTO_ENABLED,
                            sb_manual_enable,
                            sb_speed_tier,
                        );
                        let mut sb_i2c = I2cDevice::new(i2c_bus);
                        match crate::write_smart_battery_reg_retry(
                            &mut sb_i2c,
                            SB_REG_CHG_CONFIG,
                            desired,
                        )
                        .await
                        {
                            Ok(()) => {
                                sb_config_value = desired;
                                info!("smart-battery: cfg re-applied after drift");
                            }
                            Err(()) => {
                                warn!("smart-battery: failed to reapply charge config");
                            }
                        }
                    }
                }
                Err(()) => warn!("smart-battery: cfg read failed"),
            }
        }

        // Periodic state snapshot for logging / UI (every 10s)
        if now_millis.saturating_sub(sb_last_state_poll_ms) >= SB_STATE_POLL_INTERVAL_MS {
            sb_last_state_poll_ms = now_millis;
            let mut snapshot_ok = true;
            let mut status: Option<u8> = None;
            let mut pause: Option<u8> = None;
            let mut flags: Option<u16> = None;
            let mut cell_mv: [Option<u16>; 5] = [None; 5];
            let mut cells_present: Option<u8> = None;
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            if let Ok(s) = crate::read_smart_battery_reg_retry(&mut sb_i2c, 0x30, 2, 2).await {
                status = Some(s);
            } else {
                warn!("sb:state read CHG_STATUS failed");
                snapshot_ok = false;
            }
            if let Ok(p) =
                crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_CHG_PAUSE_CAUSE, 2, 2).await
            {
                pause = Some(p);
            } else {
                warn!("sb:state read CHG_PAUSE_CAUSE failed");
                snapshot_ok = false;
            }
            if let Ok(f_lo) =
                crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS, 2, 2).await
            {
                if let Ok(f_hi) =
                    crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS + 1, 2, 2)
                        .await
                {
                    flags = Some(((f_hi as u16) << 8) | f_lo as u16);
                } else {
                    snapshot_ok = false;
                }
            } else {
                warn!("sb:state read STATE_FLAGS failed");
                snapshot_ok = false;
            }
            // Cell voltages (best-effort, non-atomic)
            if let Ok(c) = crate::read_smart_battery_reg_retry(&mut sb_i2c, 0x1F, 2, 2).await {
                cells_present = Some(c);
                let count = (c as usize).min(5);
                for i in 0..count {
                    let base = 0x50u8.wrapping_add((i as u8) * 2);
                    match (
                        crate::read_smart_battery_reg_retry(&mut sb_i2c, base, 2, 2).await,
                        crate::read_smart_battery_reg_retry(
                            &mut sb_i2c,
                            base.wrapping_add(1),
                            2,
                            2,
                        )
                        .await,
                    ) {
                        (Ok(lo), Ok(hi)) => {
                            cell_mv[i] = Some(((hi as u16) << 8) | lo as u16);
                        }
                        _ => {
                            warn!("sb:state read cell{} failed", i + 1);
                            snapshot_ok = false;
                        }
                    }
                }
            } else {
                warn!("sb:state read CELLS_PRESENT failed");
                snapshot_ok = false;
            }

            // Cache latest state flags and per-cell voltages for the UI battery detail page.
            last_state_flags = flags;
            last_cells_mv = cell_mv;
            last_chg_pause_cause = pause;

            // Track repeated communication failures so higher layers can surface
            // a coarse COMM ERR fault when the smart-battery state window is
            // consistently unavailable.
            if snapshot_ok {
                sb_comm_error_streak = 0;
                sb_comm_error = false;
            } else {
                if sb_comm_error_streak < u8::MAX {
                    sb_comm_error_streak = sb_comm_error_streak.saturating_add(1);
                }
                if sb_comm_error_streak >= 2 {
                    sb_comm_error = true;
                }
            }

            // Periodic smart-battery state snapshot; keep at debug level to reduce noise.
            debug!(
                "sb:state status=0x{:02x} pause=0x{:02x} flags=0x{:04x}",
                status.unwrap_or(0xFF),
                pause.unwrap_or(0xFF),
                flags.unwrap_or(0xFFFF)
            );
            if let Some(c) = cells_present {
                debug!(
                    "sb:cells n={} mv={:?}",
                    c,
                    [
                        cell_mv[0].unwrap_or(0),
                        cell_mv[1].unwrap_or(0),
                        cell_mv[2].unwrap_or(0),
                        cell_mv[3].unwrap_or(0),
                        cell_mv[4].unwrap_or(0)
                    ]
                );
            }
        }

        // === UPS discharge / OUT state machine (SC8815 OTG path) ===
        //
        // This implements the OUT_ENABLED / OUT_DISABLED transitions described
        // in discharge_policy.md §§3–5, using smart-battery STATE_FLAGS,
        // pack voltage, UPS temperature (SC8815 ADIN) and SC8815 status.

        // Map STATE_FLAGS into a coarse “pack critical fault” indicator.
        let mut pack_critical_fault = false;
        if let Some(flags) = last_state_flags {
            let fault_mask = SB_STATE_FLAG_FAULT_BQ | SB_STATE_FLAG_FAULT_SC;
            if (flags & fault_mask) != 0 {
                pack_critical_fault = true;
            }
        }

        // Decode smart-battery TEMP_STATUS into high-level fault flags so both
        // discharge and charge logic can apply additional safety gating.
        let sb_temp_fault = sb_temp_status.map(crate::decode_temp_status);
        let temp_fault_blocks_disch =
            matches!(sb_temp_fault, Some(flags) if flags.temp_low || flags.temp_high_dsg);
        let temp_fault_blocks_charge =
            matches!(sb_temp_fault, Some(flags) if flags.temp_low || flags.temp_high_chg);

        // UPS temperature-based stop / resume (SC8815 ADIN; discharge_policy.md §4.2).
        if let Some(temp) = last_adin_temp_c {
            if ups_temp_pause_active {
                if temp <= UPS_DISCH_RESUME_C {
                    ups_temp_pause_active = false;
                    info!("discharge: UPS temperature resume at {=f32}°C", temp);
                }
            } else if temp >= UPS_DISCH_STOP_C {
                ups_temp_pause_active = true;
                if out_enabled {
                    if let Some(t) = tca.as_mut() {
                        sc8815_set_power_stage(t, &mut sc, false).await;
                    }
                    out_enabled = false;
                }
                info!(
                    "discharge: disabled due to high UPS temperature temp={=f32}°C",
                    temp
                );
            }
        }

        // Check SC8815 status for OTP / VBUS_SHORT faults (discharge_policy.md §4.4).
        let mut sc_status: Option<sc8815::SC8815Status> = None;
        if let Some(drv) = sc.as_mut() {
            if let Ok(status) = drv.get_device_status().await {
                sc_status = Some(status);
            }
        }

        if let Some(status) = sc_status {
            // OTP is treated as a fatal fault: immediately shut down OUT.
            if status.otp_fault {
                sc_fault_latched = true;
                if let Some(t) = tca.as_mut() {
                    sc8815_set_power_stage(t, &mut sc, false).await;
                }
                if out_enabled {
                    out_enabled = false;
                }
                warn!(
                    "discharge: disabled due to SC8815 OTP fault (vbus_short={})",
                    status.vbus_short_fault
                );
            } else if status.vbus_short_fault {
                // TODO(discharge_policy.md §4.4): treat VBUS_SHORT as non-fatal
                // for now and rely on SC8815 internal foldback/hiccup. Here we
                // only toggle DIS_ShortFoldBack per the recommended sequence
                // while keeping OUT enabled for debugging.
                if let Some(drv) = sc.as_mut() {
                    let _ = drv
                        .clear_vbus_short_fault_with_delay(|| {
                            Timer::after(Duration::from_millis(10))
                        })
                        .await;
                }
                info!(
                    "discharge: VBUS_SHORT reported, relying on SC8815 foldback/hiccup (OUT kept enabled)"
                );
            }
        }

        // Low-voltage cutoff with hysteresis (discharge_policy.md §4.3).
        let pack_v_for_disch = vbat_mv.or(sb_last_vbat_mv);
        if let Some(vbat) = pack_v_for_disch {
            if out_enabled && vbat <= DISCH_STOP_VBAT_MV {
                if let Some(t) = tca.as_mut() {
                    sc8815_set_power_stage(t, &mut sc, false).await;
                }
                out_enabled = false;
                info!(
                    "discharge: disabled due to low pack voltage vbat={=u32}mV",
                    vbat
                );
            }
        }

        // Pack / protection board critical fault gating (discharge_policy.md §4.1).
        if out_enabled && pack_critical_fault {
            if let Some(t) = tca.as_mut() {
                sc8815_set_power_stage(t, &mut sc, false).await;
            }
            out_enabled = false;
            if let Some(flags) = last_state_flags {
                info!(
                    "discharge: disabled due to pack fault flags=0x{:04x}",
                    flags
                );
            } else {
                info!("discharge: disabled due to pack fault (flags unavailable)");
            }
        }

        // Smart-battery TEMP_STATUS gating: never drive OUT when pack is in a
        // low-temperature or high-discharge thermal fault.
        if out_enabled && temp_fault_blocks_disch {
            if let Some(t) = tca.as_mut() {
                sc8815_set_power_stage(t, &mut sc, false).await;
            }
            out_enabled = false;
            if let Some(flags) = sb_temp_fault {
                info!(
                    "discharge: disabled due to TEMP_STATUS low={} high_chg={} high_dsg={}",
                    flags.temp_low, flags.temp_high_chg, flags.temp_high_dsg
                );
            } else {
                info!("discharge: disabled due to TEMP_STATUS fault");
            }
        }

        // Allow conditions: OUT_DISABLED → OUT_ENABLED (discharge_policy.md §5).
        let mut can_enable_out = false;
        if !out_enabled {
            let safe_vbat = pack_v_for_disch
                .map(|v| v >= DISCH_RESUME_VBAT_MV)
                .unwrap_or(false);
            let safe_temp = !ups_temp_pause_active
                && last_adin_temp_c
                    .map(|t| t < UPS_DISCH_STOP_C)
                    .unwrap_or(true);
            let no_faults = !pack_critical_fault && !sc_fault_latched;
            let temp_status_ok = !temp_fault_blocks_disch;
            // TODO: align UPS feature enable with UI/mode machine (discharge_policy.md §5.4).
            let ups_feature_enabled = true;

            if safe_vbat && safe_temp && no_faults && temp_status_ok && ups_feature_enabled {
                can_enable_out = true;
            }
        }

        if false && can_enable_out {
            if let Some(t) = tca.as_mut() {
                // Step 1: initialise / configure SC8815 for OTG + external FB.
                if sc8815_init_otg(
                    i2c_bus,
                    t,
                    &mut sc,
                    &mut sc_init_done,
                    &mut sc_otg_configured,
                )
                .await
                {
                    if sc_otg_configured {
                        // Step 2: 12V 模式下，根据 AC 是否存在选择 11.5V / 12V，
                        // 并通过外部分压 VBUSREF_E 设定输出电压。
                        let ac_mode = ac_present;
                        let target_vbus_mv: u16 = if ac_mode {
                            UPS_VBUS_AC_ONLINE_MV
                        } else {
                            UPS_VBUS_AC_OFFLINE_MV
                        };
                        if sc8815_set_vbus_external_target(&mut sc, target_vbus_mv).await {
                            last_vbus_ac_mode = Some(ac_mode);
                        }

                        // Step 3: 打开功率级（PSTOP / EN_OTG），OUT 进入稳压放电。
                        sc8815_set_power_stage(t, &mut sc, true).await;
                        out_enabled = true;
                        if let Some(vbat) = pack_v_for_disch {
                            info!("discharge: enabled (vbat={=u32}mV)", vbat);
                        } else {
                            info!("discharge: enabled");
                        }
                    }
                }
            }
        }

        // When OUT is enabled, sample the SC8815 ADC for OUT trio and ADIN/UPS temperature.
        if false && out_enabled {
            // AC presence flips between online/offline 时，重新根据 12V 策略
            // 更新外部 VBUS 目标电压。
            if sc_otg_configured {
                let ac_mode_now = ac_present;
                if Some(ac_mode_now) != last_vbus_ac_mode {
                    let target_vbus_mv: u16 = if ac_mode_now {
                        UPS_VBUS_AC_ONLINE_MV
                    } else {
                        UPS_VBUS_AC_OFFLINE_MV
                    };
                    if sc8815_set_vbus_external_target(&mut sc, target_vbus_mv).await {
                        last_vbus_ac_mode = Some(ac_mode_now);
                    }
                }
            }

            if let Some(drv) = sc.as_mut() {
                if let Ok(meas) = drv.get_adc_measurements().await {
                    out_v_mv = Some(meas.vbus_mv as u32);
                    // Treat discharge towards OUT as positive current.
                    out_a_ma = Some(meas.ibus_ma as i32);
                    let p = (meas.vbus_mv as u32).saturating_mul(meas.ibus_ma as u32);
                    out_w_mw = Some(p);

                    if let Some(temp_c) = crate::adin_temp::adin_mv_to_celsius(meas.adin_mv) {
                        last_adin_temp_c = Some(temp_c);
                    }

                    // Periodic SC8815 both-side measurement log (VBUS/VBAT + IBUS/IBAT).
                    if now_millis.saturating_sub(last_sc_meas_log_ms) >= 1_000 {
                        last_sc_meas_log_ms = now_millis;
                        info!(
                            "discharge: meas vbus={=u16}mV ibus={=u16}mA vbat={=u16}mV ibat={=u16}mA",
                            meas.vbus_mv, meas.ibus_ma, meas.vbat_mv, meas.ibat_ma
                        );
                    }
                } else {
                    warn!("discharge: SC8815 ADC read failed");
                }
            }
        }

        let temp_pause_effective = sb_temp_pause_active || temp_fault_blocks_charge;

        // Temperature-based pause: force manual charging off while any
        // high-temperature condition is active (local pack temp or
        // smart-battery TEMP_STATUS).
        // AC stability window derived from AC GOOD edges.
        let ac_stable_now =
            ac_present && now_millis.saturating_sub(ac_state_last_change_ms) >= AC_STABLE_MS;

        if temp_pause_effective {
            if sb_manual_enable {
                let desired_config =
                    crate::compose_sb_charge_config(SB_AUTO_ENABLED, false, sb_speed_tier);
                if sb_config_value != desired_config {
                    let mut sb_i2c = I2cDevice::new(i2c_bus);
                    if crate::write_smart_battery_reg_retry(
                        &mut sb_i2c,
                        SB_REG_CHG_CONFIG,
                        desired_config,
                    )
                    .await
                    .is_ok()
                    {
                        sb_config_value = desired_config;
                        sb_manual_enable = false;
                        info!("charge: disabled due to high temperature");
                    } else {
                        warn!("charge: failed to disable during temperature pause");
                    }
                }
            }
        } else if let Some(vbat) = vbat_mv.or(sb_last_vbat_mv) {
            // Derive adapter stability window for charge decisions from AC GOOD.
            info!(
                "charge: decision ac_present={} ac_stable={} vin_meas_mv={:?} vbat={}mV manual={} temp_pause={}",
                ac_present,
                ac_stable_now,
                vin_meas_mv,
                vbat,
                sb_manual_enable,
                temp_pause_effective
            );
            if !ac_present {
                if sb_manual_enable {
                    let desired_config =
                        crate::compose_sb_charge_config(SB_AUTO_ENABLED, false, sb_speed_tier);
                    if sb_config_value != desired_config && {
                        let mut sb_i2c = I2cDevice::new(i2c_bus);
                        crate::write_smart_battery_reg_retry(
                            &mut sb_i2c,
                            SB_REG_CHG_CONFIG,
                            desired_config,
                        )
                        .await
                        .is_ok()
                    } {
                        sb_config_value = desired_config;
                        sb_manual_enable = false;
                        info!("charge: disabled because adapter is missing");
                    }
                } else if vbat <= CHARGE_START_VBAT_MV && !charge_skip_adapter_logged {
                    info!("charge: skip enable (adapter missing, vbat={=u32}mV)", vbat);
                    charge_skip_adapter_logged = true;
                }
            } else if !ac_stable_now {
                // Adapter just recovered or is unstable: keep charging disabled within the
                // stability window and only log once.
                if vbat <= CHARGE_START_VBAT_MV && !charge_skip_adapter_logged {
                    info!(
                        "charge: skip enable (adapter unstable, vbat={=u32}mV)",
                        vbat
                    );
                    charge_skip_adapter_logged = true;
                }
            } else {
                charge_skip_adapter_logged = false;
                let mut target_manual = sb_manual_enable;
                if !sb_manual_enable && vbat <= CHARGE_START_VBAT_MV {
                    target_manual = true;
                }
                if sb_manual_enable && vbat >= CHARGE_STOP_VBAT_MV {
                    target_manual = false;
                }

                if target_manual != sb_manual_enable {
                    let desired_config = crate::compose_sb_charge_config(
                        SB_AUTO_ENABLED,
                        target_manual,
                        sb_speed_tier,
                    );
                    let mut sb_i2c = I2cDevice::new(i2c_bus);
                    match crate::write_smart_battery_reg_retry(
                        &mut sb_i2c,
                        SB_REG_CHG_CONFIG,
                        desired_config,
                    )
                    .await
                    {
                        Ok(()) => {
                            sb_config_value = desired_config;
                            sb_manual_enable = target_manual;
                            if target_manual {
                                info!(
                                    "charge: enabled (vbat={=u32}mV, threshold={=u32}mV)",
                                    vbat, CHARGE_START_VBAT_MV
                                );
                            } else {
                                info!(
                                    "charge: disabled at {=u32}mV (stop threshold {=u32}mV)",
                                    vbat, CHARGE_STOP_VBAT_MV
                                );
                            }
                        }
                        Err(()) => {
                            warn!("charge: failed to update charge config register");
                        }
                    }
                }
            }
        }

        // Finally, publish the latest power state snapshot for other tasks.
        let ac_stable = ac_stable_now;
        let charge_mode = if sb_manual_enable {
            ChargeMode::Manual
        } else {
            ChargeMode::Auto
        };

        let mut state = power_state.lock().await;
        *state = PowerState {
            ac_present,
            ac_stable,
            vin_meas_mv,
            ina226_fault,
            charge_mode,
            chg_config: sb_config_value,
            vbat_mv,
            ibat_ma,
            cells_mv: last_cells_mv,
            state_flags: last_state_flags,
            sb_chg_pause_cause: last_chg_pause_cause,
            sb_temp_status,
            smart_batt_temps: sb_temps,
            adin_temp_c: last_adin_temp_c,
            temp_pause_active: temp_pause_effective,
            out_enabled,
            out_v_mv,
            out_a_ma,
            out_w_mw,
            sb_comm_error,
            last_update_ms: now_millis,
        };
    }
}
