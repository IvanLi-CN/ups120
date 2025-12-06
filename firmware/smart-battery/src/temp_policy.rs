use crate::thermal::ThermalSnapshot;
use sb_temp_policy as core;

pub use core::{TempInputs, TempPolicyOutput, TempPolicyState};

/// Evaluate the unified thermal policy using the project-level ThermalSnapshot.
///
/// This is a thin adapter that maps ThermalSnapshot (used throughout the
/// smart-battery firmware) into the core TempInputs understood by the shared
/// sb_temp_policy crate.
pub fn eval(
    prev: &TempPolicyState,
    snapshot: &ThermalSnapshot,
) -> (TempPolicyState, TempPolicyOutput) {
    let inputs = TempInputs {
        // Unified policy defines T_PACK as the hottest NTC.
        t_pack_ntc_max_0_01c: snapshot.t_ntc_max_0_01c,
        t_chg_0_01c: snapshot.t_chg_0_01c,
        t_bal_0_01c: snapshot.t_bq_int_0_01c,
        t_mcu_0_01c: snapshot.t_mcu_0_01c,
    };
    core::eval(prev, &inputs)
}
