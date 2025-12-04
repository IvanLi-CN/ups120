//! Pack NTC + MCU temperature sampling for smart-battery.
//!
//! - Drives PB12 (`NTC_3V3`) as a gated supply for four 43 kΩ pull-ups.
//! - Samples ADC1 IN0..IN3 (PA0..PA3) for 4× NTC networks.
//! - Samples the internal MCU temperature sensor.
//! - Converts all readings into 0.01 °C and updates the shared
//!   [`crate::thermal`] aggregation state.

use core::ptr;

use defmt::debug;
use embassy_executor::task;
use embassy_stm32::Peri;
use embassy_stm32::adc;
use embassy_stm32::adc::{Adc, SampleTime, Temperature};
use embassy_stm32::bind_interrupts;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::{ADC1, PA0, PA1, PA2, PA3, PB12};
use embassy_time::{Duration, Timer};

use crate::thermal::{self, TEMP_INVALID_0_01C};

bind_interrupts!(struct AdcIrqs {
    ADC1_COMP => adc::InterruptHandler<ADC1>;
});

pub struct NtcTempTaskArgs {
    pub adc: Peri<'static, ADC1>,
    pub ts45: Peri<'static, PA0>,
    pub ts34: Peri<'static, PA1>,
    pub ts23: Peri<'static, PA2>,
    pub ts12: Peri<'static, PA3>,
    pub ntc_vcc: Peri<'static, PB12>,
}

// Warm-up delay for the RC network (~5× tau per design notes, ~18 ms).
const NTC_WARMUP_MS: u64 = 20;
// Nominal sampling period.
const NTC_SAMPLE_PERIOD_MS: u64 = 1000;

// STM32L051 temperature sensor calibration constants (see RM0377 + STM32L051C8 datasheet).
const TS_CAL1_ADDR: *const u16 = 0x1FF8_007A as *const u16; // 30 °C @ VDDA=3 V
const TS_CAL2_ADDR: *const u16 = 0x1FF8_007E as *const u16; // 130 °C @ VDDA=3 V
const TS_CAL1_TEMP_C: i32 = 30;
const TS_CAL2_TEMP_C: i32 = 130;

// ADC characteristics for NTC network (12-bit, VREF+=VDDA≈3.3 V).
const ADC_MAX_COUNTS: u32 = (1 << 12) - 1;
const VDDA_MV: u32 = 3300;

struct LutPoint {
    mv: u16,
    temp_0_01c: i16,
}

// 10 k / B3380 NTC with 43 k pull-up to 3.3 V (see battery_temp_sensing.md and
// ups-main ADIN LUT). Entries ordered by decreasing voltage.
const NTC_LUT: [LutPoint; 15] = [
    LutPoint {
        mv: 2098,
        temp_0_01c: -2000,
    },
    LutPoint {
        mv: 1691,
        temp_0_01c: -1000,
    },
    LutPoint {
        mv: 1308,
        temp_0_01c: 0,
    },
    LutPoint {
        mv: 983,
        temp_0_01c: 1000,
    },
    LutPoint {
        mv: 726,
        temp_0_01c: 2000,
    },
    LutPoint {
        mv: 534,
        temp_0_01c: 3000,
    },
    LutPoint {
        mv: 393,
        temp_0_01c: 4000,
    },
    LutPoint {
        mv: 291,
        temp_0_01c: 5000,
    },
    LutPoint {
        mv: 218,
        temp_0_01c: 6000,
    },
    LutPoint {
        mv: 165,
        temp_0_01c: 7000,
    },
    LutPoint {
        mv: 126,
        temp_0_01c: 8000,
    },
    LutPoint {
        mv: 98,
        temp_0_01c: 9000,
    },
    LutPoint {
        mv: 77,
        temp_0_01c: 10000,
    },
    LutPoint {
        mv: 61,
        temp_0_01c: 11000,
    },
    LutPoint {
        mv: 49,
        temp_0_01c: 12000,
    },
];

fn adc_counts_to_mv(sample: u16) -> u16 {
    ((sample as u32 * VDDA_MV) / ADC_MAX_COUNTS) as u16
}

fn ntc_mv_to_temp_0_01c(mv: u16) -> i16 {
    let mv_i = mv as i32;
    let high = &NTC_LUT[0];
    let low = &NTC_LUT[NTC_LUT.len() - 1];

    if mv_i > high.mv as i32 || mv_i < low.mv as i32 {
        return TEMP_INVALID_0_01C;
    }

    for pair in NTC_LUT.windows(2) {
        let hi = &pair[0];
        let lo = &pair[1];
        if mv_i <= hi.mv as i32 && mv_i >= lo.mv as i32 {
            let span_mv = (hi.mv - lo.mv) as i32;
            if span_mv <= 0 {
                return hi.temp_0_01c;
            }
            let delta_mv = (hi.mv as i32) - mv_i;
            let temp_span = (lo.temp_0_01c as i32) - (hi.temp_0_01c as i32);
            // Linear interpolation in integer domain with rounding.
            let temp = (hi.temp_0_01c as i32) + (temp_span * delta_mv + span_mv / 2) / span_mv;
            return temp.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    TEMP_INVALID_0_01C
}

fn adc_sample_to_ntc_temp_0_01c(sample: u16) -> i16 {
    // Treat rails as sensor fault.
    if sample == 0 || sample == u16::MAX {
        return TEMP_INVALID_0_01C;
    }
    let mv = adc_counts_to_mv(sample);
    ntc_mv_to_temp_0_01c(mv)
}

fn adc_sample_to_mcu_temp_0_01c(sample: u16, ts_cal1: u16, ts_cal2: u16) -> i16 {
    let ts_data = sample as i32;
    let cal1 = ts_cal1 as i32;
    let cal2 = ts_cal2 as i32;

    if cal2 <= cal1 {
        return TEMP_INVALID_0_01C;
    }

    let delta_temp_c = TS_CAL2_TEMP_C - TS_CAL1_TEMP_C;
    let delta_cal = cal2 - cal1;
    let delta_sample = ts_data - cal1;

    // Temperature in 0.01 °C based on RM0377 formula.
    let temp_x100 = (delta_temp_c
        .saturating_mul(delta_sample)
        .saturating_mul(100)
        / delta_cal)
        + TS_CAL1_TEMP_C * 100;

    temp_x100.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[task]
pub async fn ntc_temp_task(args: NtcTempTaskArgs) {
    let NtcTempTaskArgs {
        adc,
        mut ts45,
        mut ts34,
        mut ts23,
        mut ts12,
        ntc_vcc,
    } = args;

    let mut adc = Adc::new(adc, AdcIrqs);
    // Use a long sampling time to satisfy the high source impedance of the NTC network.
    adc.set_sample_time(SampleTime::from_bits(7));

    // Internal MCU temperature channel.
    let mut temp_chan = adc.enable_temperature();

    // Read factory calibration points once at boot.
    let ts_cal1 = unsafe { ptr::read_volatile(TS_CAL1_ADDR) };
    let ts_cal2 = unsafe { ptr::read_volatile(TS_CAL2_ADDR) };

    // PB12 powers the 4× 43 k pull-ups.
    let mut ntc_vcc = Output::new(ntc_vcc, Level::Low, Speed::Low);

    loop {
        ntc_vcc.set_high();
        Timer::after(Duration::from_millis(NTC_WARMUP_MS)).await;

        let raw0 = adc.read(&mut ts45).await;
        let raw1 = adc.read(&mut ts34).await;
        let raw2 = adc.read(&mut ts23).await;
        let raw3 = adc.read(&mut ts12).await;

        let mcu_raw = adc.read(&mut temp_chan).await;

        ntc_vcc.set_low();

        let t_ntc_0_01c = [
            adc_sample_to_ntc_temp_0_01c(raw0),
            adc_sample_to_ntc_temp_0_01c(raw1),
            adc_sample_to_ntc_temp_0_01c(raw2),
            adc_sample_to_ntc_temp_0_01c(raw3),
        ];
        let t_mcu_0_01c = adc_sample_to_mcu_temp_0_01c(mcu_raw, ts_cal1, ts_cal2);

        thermal::update_ntc_temps(&t_ntc_0_01c);
        thermal::update_mcu_temp(t_mcu_0_01c);

        debug!("ntc:t={:?} mcu={}x0.01C", t_ntc_0_01c, t_mcu_0_01c);

        Timer::after(Duration::from_millis(NTC_SAMPLE_PERIOD_MS)).await;
    }
}
