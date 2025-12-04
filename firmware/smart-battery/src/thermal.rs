use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex};

/// Sentinel value for "temperature unknown" in 0.01 °C domain.
pub const TEMP_INVALID_0_01C: i16 = i16::MIN;

/// Aggregated thermal snapshot used for I2C mirroring and diagnostics.
#[derive(Copy, Clone)]
pub struct ThermalSnapshot {
    /// Pack-level safety temperature, typically
    /// max(NTC hottest, TMP75, BQ internal, MCU).
    pub t_pack_0_01c: i16,
    /// Charger / board temperature (TMP75).
    pub t_chg_0_01c: i16,
    /// 4× pack NTC temperatures (0..3).
    pub t_ntc_0_01c: [i16; 4],
    /// Hottest and coldest NTC (for internal policy/debug).
    pub t_ntc_max_0_01c: i16,
    pub t_ntc_min_0_01c: i16,
    /// BQ76920 internal die temperature.
    pub t_bq_int_0_01c: i16,
    /// STM32 MCU on-die temperature.
    pub t_mcu_0_01c: i16,
}

impl Default for ThermalSnapshot {
    fn default() -> Self {
        Self {
            t_pack_0_01c: TEMP_INVALID_0_01C,
            t_chg_0_01c: TEMP_INVALID_0_01C,
            t_ntc_0_01c: [TEMP_INVALID_0_01C; 4],
            t_ntc_max_0_01c: TEMP_INVALID_0_01C,
            t_ntc_min_0_01c: TEMP_INVALID_0_01C,
            t_bq_int_0_01c: TEMP_INVALID_0_01C,
            t_mcu_0_01c: TEMP_INVALID_0_01C,
        }
    }
}

#[derive(Copy, Clone)]
struct ThermalState {
    t_tmp75_0_01c: i16,
    t_ntc_0_01c: [i16; 4],
    t_bq_int_0_01c: i16,
    t_mcu_0_01c: i16,
}

impl ThermalState {
    const fn new() -> Self {
        Self {
            t_tmp75_0_01c: TEMP_INVALID_0_01C,
            t_ntc_0_01c: [TEMP_INVALID_0_01C; 4],
            t_bq_int_0_01c: TEMP_INVALID_0_01C,
            t_mcu_0_01c: TEMP_INVALID_0_01C,
        }
    }
}

static THERMAL_STATE: BlockingMutex<CriticalSectionRawMutex, ThermalState> =
    BlockingMutex::new(ThermalState::new());

/// Update all four NTC temperatures (0.01 °C).
///
/// Caller should pass `TEMP_INVALID_0_01C` for channels that are not wired
/// or that yielded an out-of-range ADC code.
pub fn update_ntc_temps(t_ntc_0_01c: &[i16; 4]) {
    // Safety: this function is never called re-entrantly and only performs a
    // short critical-section update.
    unsafe {
        THERMAL_STATE.lock_mut(|state| {
            state.t_ntc_0_01c = *t_ntc_0_01c;
        });
    }
}

/// Update board/charger temperature from TMP75 (0.01 °C).
pub fn update_tmp75_temp(raw_0_01c: i16) {
    unsafe {
        THERMAL_STATE.lock_mut(|state| {
            state.t_tmp75_0_01c = raw_0_01c;
        });
    }
}

/// Update BQ76920 internal temperature (0.01 °C).
pub fn update_bq_int_temp(raw_0_01c: i16) {
    unsafe {
        THERMAL_STATE.lock_mut(|state| {
            state.t_bq_int_0_01c = raw_0_01c;
        });
    }
}

/// Update MCU on-die temperature (0.01 °C).
pub fn update_mcu_temp(raw_0_01c: i16) {
    unsafe {
        THERMAL_STATE.lock_mut(|state| {
            state.t_mcu_0_01c = raw_0_01c;
        });
    }
}

fn max_valid(a: i16, b: i16) -> i16 {
    match (a == TEMP_INVALID_0_01C, b == TEMP_INVALID_0_01C) {
        (true, true) => TEMP_INVALID_0_01C,
        (false, true) => a,
        (true, false) => b,
        (false, false) => if a > b { a } else { b },
    }
}

/// Take a snapshot of the latest thermal state, computing pack-level
/// aggregate, NTC min/max and returning a copy suitable for mirroring.
pub fn snapshot() -> ThermalSnapshot {
    THERMAL_STATE.lock(|state| {
        let mut ntc_max = TEMP_INVALID_0_01C;
        let mut ntc_min = TEMP_INVALID_0_01C;
        for &t in state.t_ntc_0_01c.iter() {
            if t == TEMP_INVALID_0_01C {
                continue;
            }
            if ntc_max == TEMP_INVALID_0_01C {
                ntc_max = t;
                ntc_min = t;
            } else {
                if t > ntc_max {
                    ntc_max = t;
                }
                if t < ntc_min {
                    ntc_min = t;
                }
            }
        }

        // Charger/board temperature (TMP75) feeds T_CHG.
        let t_chg_0_01c = state.t_tmp75_0_01c;

        // Pack safety temperature = max(NTC hottest, TMP75, BQ internal, MCU).
        let mut t_pack_0_01c = TEMP_INVALID_0_01C;
        t_pack_0_01c = max_valid(t_pack_0_01c, ntc_max);
        t_pack_0_01c = max_valid(t_pack_0_01c, t_chg_0_01c);
        t_pack_0_01c = max_valid(t_pack_0_01c, state.t_bq_int_0_01c);
        t_pack_0_01c = max_valid(t_pack_0_01c, state.t_mcu_0_01c);

        ThermalSnapshot {
            t_pack_0_01c,
            t_chg_0_01c,
            t_ntc_0_01c: state.t_ntc_0_01c,
            t_ntc_max_0_01c: ntc_max,
            t_ntc_min_0_01c: ntc_min,
            t_bq_int_0_01c: state.t_bq_int_0_01c,
            t_mcu_0_01c: state.t_mcu_0_01c,
        }
    })
}

