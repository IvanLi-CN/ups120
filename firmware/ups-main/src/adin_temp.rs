struct LutPoint {
    mv: u16,
    temp_c: f32,
}

// Lookup table generated from the Beta model (R53 = 43 kΩ, NTC = 10 kΩ @ 25 °C, β = 3380)
// at 10 °C increments between -20 °C and 120 °C. Entries are ordered by decreasing voltage.
const ADIN_LUT: [LutPoint; 15] = [
    LutPoint {
        mv: 2098,
        temp_c: -20.0,
    },
    LutPoint {
        mv: 1691,
        temp_c: -10.0,
    },
    LutPoint {
        mv: 1308,
        temp_c: 0.0,
    },
    LutPoint {
        mv: 983,
        temp_c: 10.0,
    },
    LutPoint {
        mv: 726,
        temp_c: 20.0,
    },
    LutPoint {
        mv: 534,
        temp_c: 30.0,
    },
    LutPoint {
        mv: 393,
        temp_c: 40.0,
    },
    LutPoint {
        mv: 291,
        temp_c: 50.0,
    },
    LutPoint {
        mv: 218,
        temp_c: 60.0,
    },
    LutPoint {
        mv: 165,
        temp_c: 70.0,
    },
    LutPoint {
        mv: 126,
        temp_c: 80.0,
    },
    LutPoint {
        mv: 98,
        temp_c: 90.0,
    },
    LutPoint {
        mv: 77,
        temp_c: 100.0,
    },
    LutPoint {
        mv: 61,
        temp_c: 110.0,
    },
    LutPoint {
        mv: 49,
        temp_c: 120.0,
    },
];

/// Convert SC8815 ADIN voltage (millivolts) into degrees Celsius using linear
/// interpolation over a precomputed lookup table. Returns `None` when the
/// measurement falls outside the calibrated range.
pub fn adin_mv_to_celsius(adin_mv: u16) -> Option<f32> {
    let mv = adin_mv as i32;
    let high = ADIN_LUT.first()?;
    let low = ADIN_LUT.last()?;

    if mv > high.mv as i32 || mv < low.mv as i32 {
        return None;
    }

    for pair in ADIN_LUT.windows(2) {
        let hi = &pair[0];
        let lo = &pair[1];
        if mv <= hi.mv as i32 && mv >= lo.mv as i32 {
            let span = (hi.mv - lo.mv) as f32;
            if span <= f32::EPSILON {
                return Some(hi.temp_c);
            }

            let offset = (hi.mv as f32) - (mv as f32);
            let ratio = offset / span;
            let temp = hi.temp_c + ratio * (lo.temp_c - hi.temp_c);
            return Some(temp);
        }
    }

    None
}
