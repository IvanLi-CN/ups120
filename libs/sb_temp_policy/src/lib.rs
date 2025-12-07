#![cfg_attr(not(test), no_std)]

/// Input temperatures for the unified smart-battery thermal policy.
/// All values are in 0.01 degC (centi-degrees Celsius).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct TempInputs {
    /// Pack temperature derived from the hottest NTC on the pack surface.
    pub t_pack_ntc_max_0_01c: i16,
    /// Charger / board temperature (TMP75 near SC8815).
    pub t_chg_0_01c: i16,
    /// Balancing / pack monitor temperature (BQ76920 internal).
    pub t_bal_0_01c: i16,
    /// MCU junction temperature (STM32).
    pub t_mcu_0_01c: i16,
}

/// Internal policy state used to implement hysteresis and latching.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct TempPolicyState {
    /// Latched hard over-temperature for discharge (T_MAX >= 60 degC).
    pub high_dsg_latched: bool,
    /// Latched over-temperature for charge (T_PACK >= 55 degC or T_MAX >= 60 degC).
    pub high_chg_latched: bool,
}

/// Policy output used by the smart-battery runtime.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct TempPolicyOutput {
    /// Whether charging is permitted from a thermal perspective.
    pub allow_charge: bool,
    /// Whether discharging is permitted from a thermal perspective.
    pub allow_discharge: bool,
    /// Whether cell balancing is permitted.
    pub allow_balancing: bool,
    /// Encoded TEMP_STATUS bits (to be mirrored on I2C at 0x23).
    pub temp_status_bits: u8,
}

/// TEMP_STATUS bits (see SOFTWARE_DESIGN.md Register Map).
pub mod bits {
    /// TEMP_LOW: temperature too low (affects charge and discharge).
    pub const TEMP_LOW: u8 = 1 << 0;
    /// TEMP_HIGH_CHG: charge temperature too high.
    pub const TEMP_HIGH_CHG: u8 = 1 << 1;
    /// TEMP_HIGH_DSG: discharge temperature too high (hard cut / trip).
    pub const TEMP_HIGH_DSG: u8 = 1 << 2;
}

// Thresholds and constants in 0.01 degC domain.
const LOW_LIMIT_0_01C: i16 = -10 * 100; // -10 degC
const CHG_WINDOW_HI_0_01C: i16 = 55 * 100; // 55 degC
const HARD_CUTOFF_0_01C: i16 = 60 * 100; // 60 degC
const DSG_RECOVER_MAX_0_01C: i16 = 50 * 100; // discharge recovers when T_MAX <= 50 degC
const CHG_RECOVER_MAX_0_01C: i16 = 40 * 100; // charge recovers when T_MAX <= 40 degC

// Sentinel used by the smart-battery thermal aggregation to represent
// "temperature unknown". We treat it as invalid and exclude it from
// T_MIN/T_MAX, failing closed when all sensors are invalid.
const TEMP_INVALID_0_01C: i16 = i16::MIN;

/// Evaluate the unified thermal protection policy.
///
/// The policy follows SOFTWARE_DESIGN.md "Unified Thermal Protection Policy
/// (System-Level)" and "I2C Temperature Protection Status Bits":
/// - Discharge window:  -10 degC <= all sensors <= 60 degC.
/// - Charge window:     -10 degC <= all sensors <= 55 degC.
/// - Low temperature:   T_MIN < -10 degC  -> TEMP_LOW, CHG/DSG disabled.
/// - Soft charge pause: T_PACK >= 55 degC and T_MAX < 60 degC ->
///                      TEMP_HIGH_CHG, charge disabled, discharge allowed.
/// - Hard cut:          T_MAX >= 60 degC ->
///                      TEMP_HIGH_DSG + TEMP_HIGH_CHG, both CHG/DSG disabled,
///                      latched until recovery.
/// - Recovery:          discharge when T_MAX <= 50 degC & T_MIN >= -10 degC;
///                      charge when T_MAX <= 40 degC & T_MIN >= -10 degC.
/// - Balancing:         allowed only when charge window holds and no TEMP_* bit
///                      is set.
pub fn eval(prev: &TempPolicyState, inputs: &TempInputs) -> (TempPolicyState, TempPolicyOutput) {
    // Collect valid sensor readings.
    let mut t_min: i16 = 0;
    let mut t_max: i16 = 0;
    let mut have_valid = false;

    let temps = [
        inputs.t_pack_ntc_max_0_01c,
        inputs.t_chg_0_01c,
        inputs.t_bal_0_01c,
        inputs.t_mcu_0_01c,
    ];

    for &t in &temps {
        if t == TEMP_INVALID_0_01C {
            continue;
        }
        if !have_valid {
            t_min = t;
            t_max = t;
            have_valid = true;
        } else {
            if t < t_min {
                t_min = t;
            }
            if t > t_max {
                t_max = t;
            }
        }
    }

    // Fail closed when no sensor is available: report TEMP_LOW and block
    // charge / discharge / balancing.
    if !have_valid {
        return (
            *prev,
            TempPolicyOutput {
                allow_charge: false,
                allow_discharge: false,
                allow_balancing: false,
                temp_status_bits: bits::TEMP_LOW,
            },
        );
    }

    let mut state = *prev;

    // Derived aggregates.
    let t_pack = inputs.t_pack_ntc_max_0_01c;
    let t_min_ok = t_min >= LOW_LIMIT_0_01C;

    let temp_low_active = !t_min_ok;

    // Hard discharge cut (T_MAX >= 60 degC) with 50 degC recovery threshold.
    let hard_cut_enter = t_max >= HARD_CUTOFF_0_01C;
    let hard_cut_exit = t_max <= DSG_RECOVER_MAX_0_01C && t_min_ok;

    if state.high_dsg_latched {
        if hard_cut_exit {
            state.high_dsg_latched = false;
        }
    } else if hard_cut_enter {
        state.high_dsg_latched = true;
    }

    // Charge over-temperature (soft + hard).
    // Enter when pack is hot (>=55 degC) or when the hard discharge cut has fired.
    let pack_high_for_charge = t_pack >= CHG_WINDOW_HI_0_01C && t_max < HARD_CUTOFF_0_01C;
    let chg_recover = t_max <= CHG_RECOVER_MAX_0_01C && t_min_ok;

    if state.high_chg_latched {
        if chg_recover && !state.high_dsg_latched {
            state.high_chg_latched = false;
        }
    } else if pack_high_for_charge || state.high_dsg_latched {
        state.high_chg_latched = true;
    }

    // Compose TEMP_STATUS bits.
    let mut status_bits: u8 = 0;
    if temp_low_active {
        status_bits |= bits::TEMP_LOW;
    }
    if state.high_chg_latched {
        status_bits |= bits::TEMP_HIGH_CHG;
    }
    if state.high_dsg_latched {
        status_bits |= bits::TEMP_HIGH_DSG;
    }

    // Instantaneous charge / discharge windows.
    let discharge_window_ok = t_min_ok && t_max <= HARD_CUTOFF_0_01C;
    let charge_window_ok = t_min_ok && t_max <= CHG_WINDOW_HI_0_01C;

    // Final permissions.
    let allow_discharge = discharge_window_ok && !temp_low_active && !state.high_dsg_latched;
    let allow_charge =
        charge_window_ok && !temp_low_active && !state.high_dsg_latched && !state.high_chg_latched;

    // Balancing allowed only inside the charge window and when no protections
    // are active (TEMP_STATUS == 0).
    let allow_balancing = charge_window_ok && status_bits == 0;

    let output = TempPolicyOutput {
        allow_charge,
        allow_discharge,
        allow_balancing,
        temp_status_bits: status_bits,
    };

    (state, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits;

    fn inputs_all(t_c: i16) -> TempInputs {
        let t_0_01c = t_c * 100;
        TempInputs {
            t_pack_ntc_max_0_01c: t_0_01c,
            t_chg_0_01c: t_0_01c,
            t_bal_0_01c: t_0_01c,
            t_mcu_0_01c: t_0_01c,
        }
    }

    #[test]
    fn normal_window_allows_charge_discharge_and_balancing() {
        let inputs = inputs_all(25); // 25 degC
        let state = TempPolicyState::default();
        let (_state, out) = eval(&state, &inputs);
        assert!(out.allow_charge);
        assert!(out.allow_discharge);
        assert!(out.allow_balancing);
        assert_eq!(out.temp_status_bits, 0);
    }

    #[test]
    fn low_temperature_blocks_charge_and_discharge_and_sets_temp_low() {
        let inputs = inputs_all(-15); // -15 degC
        let state = TempPolicyState::default();
        let (_state, out) = eval(&state, &inputs);
        assert!(!out.allow_charge);
        assert!(!out.allow_discharge);
        assert!(!out.allow_balancing);
        assert_eq!(out.temp_status_bits & bits::TEMP_LOW, bits::TEMP_LOW);
    }

    #[test]
    fn high_pack_temperature_pauses_charge_but_allows_discharge() {
        // Sequence: 54 -> 55 -> 56 degC on pack, others cooler.
        let mut state = TempPolicyState::default();

        // 54 degC: everything allowed, no status bits.
        let inputs_54 = TempInputs {
            t_pack_ntc_max_0_01c: 54 * 100,
            t_chg_0_01c: 30 * 100,
            t_bal_0_01c: 30 * 100,
            t_mcu_0_01c: 30 * 100,
        };
        let (s1, out_54) = eval(&state, &inputs_54);
        state = s1;
        assert!(out_54.allow_charge);
        assert!(out_54.allow_discharge);
        assert_eq!(out_54.temp_status_bits, 0);

        // 55 degC: charge paused, discharge still allowed, TEMP_HIGH_CHG set.
        let inputs_55 = TempInputs {
            t_pack_ntc_max_0_01c: 55 * 100,
            ..inputs_54
        };
        let (s2, out_55) = eval(&state, &inputs_55);
        state = s2;
        assert!(!out_55.allow_charge);
        assert!(out_55.allow_discharge);
        assert_eq!(
            out_55.temp_status_bits & bits::TEMP_HIGH_CHG,
            bits::TEMP_HIGH_CHG
        );
        assert_eq!(out_55.temp_status_bits & bits::TEMP_HIGH_DSG, 0);

        // 56 degC: still paused, discharge allowed, TEMP_HIGH_CHG latched.
        let inputs_56 = TempInputs {
            t_pack_ntc_max_0_01c: 56 * 100,
            ..inputs_54
        };
        let (_s3, out_56) = eval(&state, &inputs_56);
        assert!(!out_56.allow_charge);
        assert!(out_56.allow_discharge);
        assert_eq!(
            out_56.temp_status_bits & bits::TEMP_HIGH_CHG,
            bits::TEMP_HIGH_CHG
        );
        assert_eq!(out_56.temp_status_bits & bits::TEMP_HIGH_DSG, 0);
    }

    #[test]
    fn hard_cut_at_60c_blocks_both_and_sets_high_bits() {
        let mut state = TempPolicyState::default();
        let inputs_60 = inputs_all(60); // 60 degC on all sensors
        let (s1, out) = eval(&state, &inputs_60);
        state = s1;
        assert!(!out.allow_charge);
        assert!(!out.allow_discharge);
        assert!(!out.allow_balancing);
        assert_eq!(
            out.temp_status_bits & bits::TEMP_HIGH_DSG,
            bits::TEMP_HIGH_DSG
        );
        assert_eq!(
            out.temp_status_bits & bits::TEMP_HIGH_CHG,
            bits::TEMP_HIGH_CHG
        );

        // Cooling to 55 degC keeps both protections latched.
        let (state, out_55) = eval(&state, &inputs_all(55));
        assert!(!out_55.allow_charge);
        assert!(!out_55.allow_discharge);
        assert!(!out_55.allow_balancing);
        assert_eq!(
            out_55.temp_status_bits & bits::TEMP_HIGH_DSG,
            bits::TEMP_HIGH_DSG
        );
        assert_eq!(
            out_55.temp_status_bits & bits::TEMP_HIGH_CHG,
            bits::TEMP_HIGH_CHG
        );

        // Cooling to 50 degC: discharge recovers, TEMP_HIGH_DSG clears but
        // TEMP_HIGH_CHG remains latched, charge still blocked.
        let (state, out_50) = eval(&state, &inputs_all(50));
        assert!(out_50.allow_discharge);
        assert!(!out_50.allow_charge);
        assert!(!out_50.allow_balancing);
        assert_eq!(out_50.temp_status_bits & bits::TEMP_HIGH_DSG, 0);
        assert_eq!(
            out_50.temp_status_bits & bits::TEMP_HIGH_CHG,
            bits::TEMP_HIGH_CHG
        );

        // Cooling to 40 degC: charge recovers, all TEMP bits clear.
        let (_state, out_40) = eval(&state, &inputs_all(40));
        assert!(out_40.allow_discharge);
        assert!(out_40.allow_charge);
        assert!(out_40.allow_balancing);
        assert_eq!(out_40.temp_status_bits, 0);
    }

    #[test]
    fn balancing_is_blocked_during_any_temperature_protection() {
        // High temperature case.
        let inputs = inputs_all(58);
        let state = TempPolicyState::default();
        let (_state, out) = eval(&state, &inputs);
        assert!(!out.allow_balancing);
        assert_ne!(out.temp_status_bits, 0);

        // Low temperature case.
        let inputs_low = inputs_all(-20);
        let state = TempPolicyState::default();
        let (_state, out_low) = eval(&state, &inputs_low);
        assert!(!out_low.allow_balancing);
        assert_ne!(out_low.temp_status_bits, 0);
    }
}
