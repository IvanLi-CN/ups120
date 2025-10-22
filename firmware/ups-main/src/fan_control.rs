use defmt::{error, info, warn};
use embedded_hal::delay::DelayNs;
use esp_hal::{
    delay::Delay,
    gpio::Output,
    ledc::{
        channel::{self, ChannelIFace},
        LowSpeed,
    },
};

use crate::tsens;

const SAMPLE_PERIOD_MS: u32 = 500;
const LOG_INTERVAL_TICKS: u8 = 4; // 500 ms * 4 = 2 s
const FILTER_WINDOW: usize = 3;

const TARGET_TEMP_C: f32 = 45.0;
const VIN_ON_THRESHOLD_C: f32 = 35.0;
const NOVIN_ON_THRESHOLD_C: f32 = 40.0;
const HYSTERESIS_C: f32 = 3.0;
const OVERTEMP_C: f32 = 80.0;

// fan_control_spec.md §3.2: PWM duty is inversely proportional to FAN_VCC
// (0% ≈ 5 V, 100% ≈ 1 V). Lower duty means stronger cooling.
const MIN_ACTIVE_DUTY: u8 = 96;
const DUTY_MAX: u8 = 100;
const DUTY_MIN: u8 = 0;
const SAFE_MODE_DUTY: u8 = 50;
const DUTY_SLEW_LIMIT: i8 = 5;

const PI_KP: f32 = 0.6;
const PI_KI: f32 = 0.08;
const INTEGRAL_LIMIT: f32 = 60.0;

const VOUT_BASE: f32 = 4.96;
const VOUT_GAIN: f32 = 3.98;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Safe,
    Overtemp,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultReason {
    TsensFault,
}

struct ControlOutcome {
    enabled: bool,
    duty: u8,
    immediate: bool,
    pi_active: bool,
}

pub struct FanController<'a> {
    fan_pwm: channel::Channel<'a, LowSpeed>,
    fan_en: Output<'a>,
    delta_c: f32,
    vin_present: bool,
    log_countdown: u8,
    mode: Mode,
    fault: Option<FaultReason>,
    filtered_temp: f32,
    last_reading: Option<tsens::Reading>,
    filter_buf: [f32; FILTER_WINDOW],
    filter_count: usize,
    filter_index: usize,
    fan_enabled: bool,
    duty_pct: u8,
    pi_integral: f32,
}

impl<'a> FanController<'a> {
    pub fn new(
        fan_pwm: channel::Channel<'a, LowSpeed>,
        mut fan_en: Output<'a>,
        delta_opt: Option<f32>,
        vin_present: bool,
    ) -> Self {
        let delta_c = delta_opt.unwrap_or(0.0);
        // ChannelIFace::set_duty uses &self, so the owned channel can set idle duty pre-transfer.
        if fan_pwm.set_duty(DUTY_MAX).is_err() {
            warn!("fan.init: failed to set idle duty");
        }
        fan_en.set_low();
        if delta_opt.is_none() {
            warn!("fan.init: tsens calibration missing -> delta defaults to 0.0°C");
        }
        Self {
            fan_pwm,
            fan_en,
            delta_c,
            vin_present,
            log_countdown: LOG_INTERVAL_TICKS,
            mode: Mode::Normal,
            fault: None,
            filtered_temp: 0.0,
            last_reading: None,
            filter_buf: [0.0; FILTER_WINDOW],
            filter_count: 0,
            filter_index: 0,
            fan_enabled: false,
            duty_pct: DUTY_MAX,
            pi_integral: 0.0,
        }
    }

    pub fn run(mut self, delay: &mut Delay) -> ! {
        loop {
            let reading = tsens::read_celsius(delay);
            let corrected = reading.base_celsius - self.delta_c;
            let filtered = self.push_sample(corrected);
            self.filtered_temp = filtered;
            self.last_reading = Some(reading);

            self.update_outputs(filtered, &reading);
            self.maybe_log();

            delay.delay_ms(SAMPLE_PERIOD_MS);
        }
    }

    fn push_sample(&mut self, value: f32) -> f32 {
        self.filter_buf[self.filter_index] = value;
        self.filter_index = (self.filter_index + 1) % FILTER_WINDOW;
        if self.filter_count < FILTER_WINDOW {
            self.filter_count += 1;
        }

        if self.filter_count < FILTER_WINDOW {
            value
        } else {
            median3(self.filter_buf)
        }
    }

    fn normal_strategy(&mut self, temperature: f32) -> ControlOutcome {
        let (on_base, off_base) = if self.vin_present {
            (VIN_ON_THRESHOLD_C, VIN_ON_THRESHOLD_C - HYSTERESIS_C)
        } else {
            (NOVIN_ON_THRESHOLD_C, NOVIN_ON_THRESHOLD_C - HYSTERESIS_C)
        };

        let on_threshold = on_base + HYSTERESIS_C;
        let off_threshold = off_base;

        if !self.fan_enabled {
            if temperature >= on_threshold {
                self.pi_integral = 0.0;
                let pi_needed = self.pi_required(temperature);
                let duty = if pi_needed {
                    self.pi_step(temperature)
                } else {
                    MIN_ACTIVE_DUTY
                };
                ControlOutcome {
                    enabled: true,
                    duty,
                    immediate: true,
                    pi_active: pi_needed,
                }
            } else {
                ControlOutcome {
                    enabled: false,
                    duty: DUTY_MAX,
                    immediate: true,
                    pi_active: false,
                }
            }
        } else if temperature <= off_threshold {
            self.pi_integral = 0.0;
            ControlOutcome {
                enabled: false,
                duty: DUTY_MAX,
                immediate: true,
                pi_active: false,
            }
        } else if self.pi_required(temperature) {
            let duty = self.pi_step(temperature);
            ControlOutcome {
                enabled: true,
                duty,
                immediate: false,
                pi_active: true,
            }
        } else {
            ControlOutcome {
                enabled: true,
                duty: MIN_ACTIVE_DUTY,
                immediate: false,
                pi_active: false,
            }
        }
    }

    fn pi_required(&self, temperature: f32) -> bool {
        !self.vin_present || temperature >= TARGET_TEMP_C
    }

    fn pi_step(&mut self, temperature: f32) -> u8 {
        // fan_control_spec.md §3.2: when T > TARGET, we must reduce duty (raise VOUT).
        // Using (TARGET - temp) keeps positive error → higher duty, negative → lower duty.
        let error = TARGET_TEMP_C - temperature;
        let dt = SAMPLE_PERIOD_MS as f32 / 1000.0;
        self.pi_integral += error * PI_KI * dt;
        if self.pi_integral > INTEGRAL_LIMIT {
            self.pi_integral = INTEGRAL_LIMIT;
        } else if self.pi_integral < -INTEGRAL_LIMIT {
            self.pi_integral = -INTEGRAL_LIMIT;
        }

        let control = PI_KP * error + self.pi_integral;
        let mut duty = MIN_ACTIVE_DUTY as f32 + control;
        // fan_control_spec.md §3.2: keep duty ≥96% to guarantee fan spin-up stability.
        if duty > MIN_ACTIVE_DUTY as f32 {
            duty = MIN_ACTIVE_DUTY as f32;
        }
        if duty < DUTY_MIN as f32 {
            duty = DUTY_MIN as f32;
        }
        (duty + 0.5) as u8
    }

    fn apply_state(&mut self, enabled: bool, target_duty: u8, immediate: bool) {
        if !enabled {
            if self.fan_enabled {
                if self.fan_pwm.set_duty(DUTY_MAX).is_err() {
                    warn!("fan.set_duty idle failure");
                }
                self.fan_en.set_low();
                self.fan_enabled = false;
                self.duty_pct = DUTY_MAX;
            }
            return;
        }

        if !self.fan_enabled {
            self.fan_en.set_high();
            self.fan_enabled = true;
        }

        let duty = if immediate {
            target_duty
        } else {
            self.slew_limit(target_duty)
        };

        if self.fan_pwm.set_duty(duty).is_err() {
            warn!("fan.set_duty failure");
        }
        self.duty_pct = duty;
    }

    fn slew_limit(&self, target: u8) -> u8 {
        let current = self.duty_pct as i16;
        let target = target as i16;
        let diff = target - current;
        let limited = diff.clamp(-DUTY_SLEW_LIMIT as i16, DUTY_SLEW_LIMIT as i16);
        (current + limited).clamp(DUTY_MIN as i16, DUTY_MAX as i16) as u8
    }

    fn update_outputs(&mut self, temperature: f32, reading: &tsens::Reading) {
        let mut fault = None;
        if reading.raw == 0 || !temperature.is_finite() {
            fault = Some(FaultReason::TsensFault);
        }

        let previous_mode = self.mode;
        let mut outcome = ControlOutcome {
            enabled: false,
            duty: DUTY_MAX,
            immediate: true,
            pi_active: false,
        };

        if let Some(reason) = fault {
            outcome.enabled = true;
            outcome.duty = SAFE_MODE_DUTY;
            outcome.immediate = true;
            outcome.pi_active = false;
            self.mode = Mode::Safe;
            self.fault = Some(reason);
        } else if temperature >= OVERTEMP_C {
            outcome.enabled = true;
            outcome.duty = DUTY_MAX;
            outcome.immediate = true;
            outcome.pi_active = false;
            self.mode = Mode::Overtemp;
            self.fault = None;
        } else {
            outcome = self.normal_strategy(temperature);
            self.mode = Mode::Normal;
            self.fault = None;
        }

        if !outcome.pi_active {
            self.pi_integral = 0.0;
        }

        self.apply_state(outcome.enabled, outcome.duty, outcome.immediate);

        if self.mode != previous_mode {
            match self.mode {
                Mode::Safe => warn!(
                    "fan.safe_mode reason={} raw={}",
                    fault_reason_str(self.fault),
                    reading.raw
                ),
                Mode::Overtemp => error!(
                    "fan.overheat temp={=f32}°C forcing={}%%",
                    temperature, DUTY_MAX
                ),
                Mode::Normal => {}
            }
        }
    }

    fn maybe_log(&mut self) {
        if self.log_countdown > 0 {
            self.log_countdown -= 1;
        }

        if self.log_countdown == 0 {
            self.log_countdown = LOG_INTERVAL_TICKS;

            if let Some(reading) = self.last_reading {
                let duty_report = if self.fan_enabled { self.duty_pct } else { 0 };
                let vout = if self.fan_enabled {
                    estimate_vout(self.duty_pct)
                } else {
                    0.0
                };
                info!(
                    "fan.report TEMP={=f32}°C RAW={=u8} ATTR={=u8} DELTA={=f32}°C DUTY={=u8}% MODE={} VOUT≈{=f32}V",
                    self.filtered_temp,
                    reading.raw,
                    reading.dac,
                    self.delta_c,
                    duty_report,
                    mode_str(self.mode),
                    vout
                );
            }
        }
    }
}

fn median3(values: [f32; FILTER_WINDOW]) -> f32 {
    let mut a = values[0];
    let mut b = values[1];
    let mut c = values[2];

    if a > b {
        core::mem::swap(&mut a, &mut b);
    }
    if b > c {
        core::mem::swap(&mut b, &mut c);
    }
    if a > b {
        core::mem::swap(&mut a, &mut b);
    }

    b
}

fn fault_reason_str(reason: Option<FaultReason>) -> &'static str {
    match reason {
        Some(FaultReason::TsensFault) => "tsens_fault",
        None => "none",
    }
}

fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Safe => "SAFE",
        Mode::Overtemp => "OVERTEMP",
    }
}

fn estimate_vout(duty: u8) -> f32 {
    let duty_fraction = duty as f32 / 100.0;
    let vout = VOUT_BASE - VOUT_GAIN * duty_fraction;
    if vout < 0.0 {
        0.0
    } else {
        vout
    }
}
