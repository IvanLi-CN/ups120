use defmt::{Debug2Format, warn};
use esp_hal::{
    ledc::{
        LowSpeed,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    time::Rate,
};

// Default duty is intentionally conservative to reduce stress and audible
// harshness. Tune per-board if needed.
pub const DEFAULT_DUTY_PCT: u8 = 8;
pub const DEFAULT_FREQ_HZ: u32 = 2_700;

#[derive(Clone, Copy)]
pub struct TimerProxy {
    number: timer::Number,
    duty: timer::config::Duty,
    frequency_hz: u32,
}

impl TimerProxy {
    pub const fn new(number: timer::Number, duty: timer::config::Duty, frequency_hz: u32) -> Self {
        Self {
            number,
            duty,
            frequency_hz,
        }
    }
}

impl TimerIFace<LowSpeed> for TimerProxy {
    fn freq(&self) -> Option<Rate> {
        Some(Rate::from_hz(self.frequency_hz))
    }

    fn configure(
        &mut self,
        _config: timer::config::Config<<LowSpeed as timer::TimerSpeed>::ClockSourceType>,
    ) -> Result<(), timer::Error> {
        Ok(())
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn duty(&self) -> Option<timer::config::Duty> {
        Some(self.duty)
    }

    fn number(&self) -> timer::Number {
        self.number
    }

    fn frequency(&self) -> u32 {
        self.frequency_hz
    }
}

pub static BUZZER_TIMER_PROXY: TimerProxy = TimerProxy::new(
    timer::Number::Timer1,
    timer::config::Duty::Duty8Bit,
    DEFAULT_FREQ_HZ,
);

pub struct Buzzer {
    timer: timer::Timer<'static, LowSpeed>,
    channel: channel::Channel<'static, LowSpeed>,
    duty_pct: u8,
    freq_hz: u32,
}

impl Buzzer {
    pub fn new(
        timer: timer::Timer<'static, LowSpeed>,
        channel: channel::Channel<'static, LowSpeed>,
    ) -> Self {
        let mut this = Self {
            timer,
            channel,
            duty_pct: DEFAULT_DUTY_PCT,
            freq_hz: DEFAULT_FREQ_HZ,
        };
        this.stop();
        this
    }

    pub fn stop(&mut self) {
        let _ = self.channel.set_duty(0);
    }

    pub fn start_tone(&mut self, freq_hz: u32, duty_pct: Option<u8>) {
        let freq_hz = freq_hz.max(1);
        let duty_pct = duty_pct.unwrap_or(self.duty_pct).min(100);
        self.freq_hz = freq_hz;
        self.duty_pct = duty_pct;

        if let Err(err) = self.timer.configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(freq_hz),
        }) {
            warn!("buzzer: timer configure failed: {}", Debug2Format(&err));
            let _ = self.channel.set_duty(0);
            return;
        }

        if let Err(err) = self.channel.set_duty(duty_pct) {
            warn!("buzzer: set duty failed: {}", Debug2Format(&err));
            let _ = self.channel.set_duty(0);
        }
    }
}
