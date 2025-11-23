#![allow(dead_code)]

/// Temporary linear SoC estimation from pack voltage.
/// - `v_mv`: pack voltage in millivolts
/// - `cells`: number of series cells (e.g., 12 for a 48V nominal pack)
/// - `empty_mv_pc`: per-cell voltage considered 0% (e.g., 3200 mV)
/// - `full_mv_pc`: per-cell voltage considered 100% (e.g., 4200 mV)
/// Returns 0..=100 (clamped)
pub fn estimate_soc_linear(v_mv: u32, cells: u8, empty_mv_pc: u16, full_mv_pc: u16) -> u8 {
    if cells == 0 || empty_mv_pc == 0 || full_mv_pc <= empty_mv_pc {
        return 0;
    }
    let cells_u32 = cells as u32;
    let v_cell_mv = v_mv / cells_u32;
    let empty = empty_mv_pc as u32;
    let full = full_mv_pc as u32;
    let span = full - empty;
    let val = if v_cell_mv <= empty {
        0
    } else if v_cell_mv >= full {
        100
    } else {
        ((v_cell_mv - empty) * 100) / span
    };
    val as u8
}

/// Convenience for 12S Li-ion packs (nominal ~44.4V, full ~50.4V).
/// Uses 3.20V/cell as empty and 4.20V/cell as full.
pub fn estimate_soc_12s_li_ion(v_mv: u32) -> u8 {
    estimate_soc_linear(v_mv, 12, 3200, 4200)
}
