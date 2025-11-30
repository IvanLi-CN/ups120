use defmt::{debug, info, warn, Debug2Format};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use esp_hal::time::Instant;
use static_cell::StaticCell;

use crate::{
    fan_control, io_expander::Tca6408a, I2cBusMutex, SharedI2cDevice, AC_STABLE_MS,
    CHARGE_START_VBAT_MV, CHARGE_STOP_VBAT_MV, SB_REG_CHG_CONFIG, SB_REG_CHG_PAUSE_CAUSE,
    SB_REG_STATE_FLAGS, SB_STATE_FLAG_AC_PRESENT, SB_STATE_POLL_INTERVAL_MS,
};

/// Charging mode exposed to other tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeMode {
    Auto,
    Manual,
}

/// Snapshot of the current power / smart-battery state.
///
/// This is intentionally a plain, `Copy`-friendly structure so UI and
/// thermal tasks can take cheap snapshots without holding the mutex
/// longer than necessary.
#[derive(Clone, Copy)]
pub struct PowerState {
    /// Raw IN_PG-derived adapter presence (last read).
    pub ac_present: bool,
    /// Whether VIN has been stable for at least `AC_STABLE_MS`.
    pub ac_stable: bool,
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
    /// Latest smart-battery temperature set (pack + charger).
    pub smart_batt_temps: Option<fan_control::SmartBatteryTemps>,
    /// Latest SC8815 ADIN-derived UPS temperature in °C.
    pub adin_temp_c: Option<f32>,
    /// Whether temperature gating has currently paused charging.
    pub temp_pause_active: bool,
    /// Millisecond timestamp (since boot) of the last successful update.
    pub last_update_ms: u64,
}

impl core::fmt::Debug for PowerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PowerState")
            .field("ac_present", &self.ac_present)
            .field("ac_stable", &self.ac_stable)
            .field("charge_mode", &self.charge_mode)
            .field("chg_config", &self.chg_config)
            .field("vbat_mv", &self.vbat_mv)
            .field("ibat_ma", &self.ibat_ma)
            .field("cells_mv", &self.cells_mv)
            .field("state_flags", &self.state_flags)
            .field("smart_batt_temps", &Debug2Format(&self.smart_batt_temps))
            .field("adin_temp_c", &self.adin_temp_c)
            .field("temp_pause_active", &self.temp_pause_active)
            .field("last_update_ms", &self.last_update_ms)
            .finish()
    }
}

impl Default for PowerState {
    fn default() -> Self {
        Self {
            ac_present: false,
            ac_stable: false,
            charge_mode: ChargeMode::Auto,
            chg_config: 0,
            vbat_mv: None,
            ibat_ma: None,
            cells_mv: [None; 5],
            state_flags: None,
            smart_batt_temps: None,
            adin_temp_c: None,
            temp_pause_active: false,
            last_update_ms: 0,
        }
    }
}

pub type PowerStateMutex = Mutex<NoopRawMutex, PowerState>;

static POWER_STATE: StaticCell<PowerStateMutex> = StaticCell::new();

/// Initialise the global power-state mutex.
///
/// This must be called once from `main` before spawning `power_task`.
pub fn init_power_state() -> &'static PowerStateMutex {
    POWER_STATE.init(Mutex::new(PowerState::default()))
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
    let mut vin_present = true;
    // Track last time IN_PG changed so we can derive a “stable for AC_STABLE_MS” window.
    let mut vin_state_last_change_ms: u64 =
        Instant::now().duration_since_epoch().as_millis() as u64;
    let mut last_in_pg_logged: Option<bool> = None;
    let mut in_pg_read_failed = false;
    let mut charge_skip_adapter_logged = false;
    if let Some(t) = tca.as_mut() {
        match t.init().await {
            Ok(()) => info!("tca6408a: init ok (CE=high, PSTOP=high)"),
            Err(_) => warn!("tca6408a: init failed (safe state not verified)"),
        }
        match t.read_in_pg().await {
            Ok(pg) => {
                vin_present = pg;
                vin_state_last_change_ms = Instant::now().duration_since_epoch().as_millis() as u64;
                last_in_pg_logged = Some(pg);
                info!("tca6408a: IN_PG={}", if pg { "high" } else { "low" });
            }
            Err(_) => {
                warn!("tca6408a: read IN_PG failed");
                in_pg_read_failed = true;
            }
        }
    }

    let mut sc: Option<sc8815::SC8815<SharedI2cDevice<'static>>> = None;
    let mut sc_init_done: bool = false;
    let mut last_adin_temp_c: Option<f32> = None;

    crate::log_sc8815_temperature(
        i2c_bus,
        &mut tca,
        &mut sc,
        &mut sc_init_done,
        &mut last_adin_temp_c,
    )
    .await;

    // Periodic loop matching the original 500 ms cadence for power sampling and
    // charger control.
    loop {
        Timer::after(Duration::from_millis(500)).await;

        // Update ADIN-derived UPS temperature via SC8815.
        crate::log_sc8815_temperature(
            i2c_bus,
            &mut tca,
            &mut sc,
            &mut sc_init_done,
            &mut last_adin_temp_c,
        )
        .await;

        // Smart-battery temperatures (pack + charger).
        let sb_temps: Option<fan_control::SmartBatteryTemps> = {
            let mut sb_i2c = I2cDevice::new(i2c_bus);
            crate::read_smart_battery_temperatures(&mut sb_i2c).await
        };

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

        // Refresh adapter presence via IN_PG.
        if let Some(t) = tca.as_mut() {
            match t.read_in_pg().await {
                Ok(state) => {
                    if in_pg_read_failed {
                        info!("power: IN_PG read recovered");
                        in_pg_read_failed = false;
                    }
                    if vin_present != state {
                        vin_present = state;
                        // Record the moment of the edge so that charging logic can
                        // enforce a “VIN stable for AC_STABLE_MS” window on resume.
                        vin_state_last_change_ms = now_millis;
                        if state {
                            charge_skip_adapter_logged = false;
                        }
                    }
                    if last_in_pg_logged != Some(state) {
                        let mut sb_i2c = I2cDevice::new(i2c_bus);
                        match crate::read_smart_battery_state_flags(&mut sb_i2c).await {
                            Some(flags) => {
                                let stm_ac = (flags & SB_STATE_FLAG_AC_PRESENT) != 0;
                                info!(
                                    "power: adapter {} (stm_ac={} flags=0x{:04x})",
                                    if state { "present" } else { "missing" },
                                    stm_ac,
                                    flags
                                );
                            }
                            None => info!(
                                "power: adapter {} (stm_ac=? read_fail)",
                                if state { "present" } else { "missing" }
                            ),
                        }
                        last_in_pg_logged = Some(state);
                    }
                }
                Err(_) => {
                    if !in_pg_read_failed {
                        warn!("tca6408a: read IN_PG failed");
                        in_pg_read_failed = true;
                    }
                }
            }
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
            }
            if let Ok(p) =
                crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_CHG_PAUSE_CAUSE, 2, 2).await
            {
                pause = Some(p);
            } else {
                warn!("sb:state read CHG_PAUSE_CAUSE failed");
            }
            if let Ok(f_lo) =
                crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS, 2, 2).await
            {
                if let Ok(f_hi) =
                    crate::read_smart_battery_reg_retry(&mut sb_i2c, SB_REG_STATE_FLAGS + 1, 2, 2)
                        .await
                {
                    flags = Some(((f_hi as u16) << 8) | f_lo as u16);
                }
            } else {
                warn!("sb:state read STATE_FLAGS failed");
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
                        }
                    }
                }
            } else {
                warn!("sb:state read CELLS_PRESENT failed");
            }

            // Cache latest state flags and per-cell voltages for the UI battery detail page.
            last_state_flags = flags;
            last_cells_mv = cell_mv;

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

        // Temperature-based pause: force manual charging off while the
        // high-temperature condition is active.
        if sb_temp_pause_active {
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
            // Derive adapter stability window for charge decisions.
            let vin_ok_for_charge =
                vin_present && now_millis.saturating_sub(vin_state_last_change_ms) >= AC_STABLE_MS;
            info!(
                "charge: decision vin_present={} vin_ok_for_charge={} vbat={}mV manual={} temp_pause={}",
                vin_present, vin_ok_for_charge, vbat, sb_manual_enable, sb_temp_pause_active
            );
            if !vin_present {
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
            } else if !vin_ok_for_charge {
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
        let ac_stable =
            vin_present && now_millis.saturating_sub(vin_state_last_change_ms) >= AC_STABLE_MS;
        let charge_mode = if sb_manual_enable {
            ChargeMode::Manual
        } else {
            ChargeMode::Auto
        };

        let mut state = power_state.lock().await;
        *state = PowerState {
            ac_present: vin_present,
            ac_stable,
            charge_mode,
            chg_config: sb_config_value,
            vbat_mv,
            ibat_ma,
            cells_mv: last_cells_mv,
            state_flags: last_state_flags,
            smart_batt_temps: sb_temps,
            adin_temp_c: last_adin_temp_c,
            temp_pause_active: sb_temp_pause_active,
            last_update_ms: now_millis,
        };
    }
}
