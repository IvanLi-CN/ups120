use chrono::{DateTime, Local};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    start: Instant,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
    pub fn now(&self) -> Timestamp {
        Timestamp {
            wall: Local::now(),
            mono_ms: self.start.elapsed(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Timestamp {
    pub wall: DateTime<Local>,
    pub mono_ms: Duration,
}

impl Timestamp {
    pub fn iso(&self) -> String {
        self.wall
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }
    pub fn mono_ms(&self) -> u128 {
        self.mono_ms.as_millis()
    }
}
