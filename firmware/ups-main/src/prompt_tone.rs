use embassy_executor::task;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{Channel, Receiver, Sender},
};
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use crate::buzzer::Buzzer;

pub const TONE_REQUEST_CAPACITY: usize = 16;

pub type ToneRequestSender = Sender<'static, NoopRawMutex, ToneRequest, TONE_REQUEST_CAPACITY>;
pub type ToneRequestReceiver = Receiver<'static, NoopRawMutex, ToneRequest, TONE_REQUEST_CAPACITY>;

static TONE_REQUEST_CHANNEL: StaticCell<
    Channel<NoopRawMutex, ToneRequest, TONE_REQUEST_CAPACITY>,
> = StaticCell::new();

pub fn channel() -> (ToneRequestSender, ToneRequestReceiver) {
    let ch = TONE_REQUEST_CHANNEL.init(Channel::new());
    (ch.sender(), ch.receiver())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SoundId {
    // Action
    ActionOk,
    ActionFail,
    ActionFault,

    // ModeMelody
    MelodyModeReady,
    MelodyModeCharge,
    MelodyModeDischarge,
    MelodyModeLowbatt,
    MelodyAcLost,
    MelodyAcRestored,

    // NoticeOnce
    NoticeInfoOnce,
    NoticeWarnOnce,
    NoticeErrorOnce,

    // AlarmLoop
    AlarmLatchedLoop,
    AlarmThermalLoop,
    AlarmCommLoop,
    AlarmLowbattLoop,
}

#[derive(Clone, Copy)]
pub enum ToneRequest {
    Action(SoundId),
    ModeMelody(SoundId),
    NoticeOnce(SoundId),
    AlarmLoopEnter(SoundId),
    AlarmLoopExit(SoundId),
}

#[derive(Clone, Copy)]
struct Tone {
    freq_hz: u32,
    duty_pct: u8,
}

#[derive(Clone, Copy)]
struct Step {
    duration_ms: u32,
    tone: Option<Tone>,
}

#[derive(Clone, Copy)]
struct Pattern {
    steps: &'static [Step],
    looped: bool,
}

const DUTY_PCT_DEFAULT: u8 = 8;
const ACTION_MIN_INTERVAL_MS: u64 = 160;

const FREQ_ACTION_OK_HZ: u32 = 2_400;
const FREQ_ACTION_FAIL_HZ: u32 = 2_000;
const FREQ_ACTION_FAULT_HZ: u32 = 1_600;
const FREQ_ALARM_HZ: u32 = 2_000;
const FREQ_NOTICE_HZ: u32 = 2_000;

const ACTION_OK: [Step; 1] = [Step {
    duration_ms: 40,
    tone: Some(Tone {
        freq_hz: FREQ_ACTION_OK_HZ,
        duty_pct: DUTY_PCT_DEFAULT,
    }),
}];

const ACTION_FAIL: [Step; 3] = [
    Step {
        duration_ms: 40,
        tone: Some(Tone {
            freq_hz: FREQ_ACTION_FAIL_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 50,
        tone: None,
    },
    Step {
        duration_ms: 40,
        tone: Some(Tone {
            freq_hz: FREQ_ACTION_FAIL_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
];

const ACTION_FAULT: [Step; 6] = [
    Step {
        duration_ms: 50,
        tone: Some(Tone {
            freq_hz: FREQ_ACTION_FAULT_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 50,
        tone: None,
    },
    Step {
        duration_ms: 50,
        tone: Some(Tone {
            freq_hz: FREQ_ACTION_FAULT_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 50,
        tone: None,
    },
    Step {
        duration_ms: 50,
        tone: Some(Tone {
            freq_hz: FREQ_ACTION_FAULT_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 50,
        tone: None,
    },
];

const ALARM_LOWBATT_LOOP: [Step; 4] = [
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 7_400,
        tone: None,
    },
];

const ALARM_THERMAL_LOOP: [Step; 4] = [
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_400,
        tone: None,
    },
];

const ALARM_LATCHED_LOOP: [Step; 4] = [
    Step {
        duration_ms: 250,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 150,
        tone: None,
    },
    Step {
        duration_ms: 250,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 350,
        tone: None,
    },
];

const ALARM_COMM_LOOP: [Step; 7] = [
    Step {
        duration_ms: 120,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 120,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 120,
        tone: Some(Tone {
            freq_hz: FREQ_ALARM_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 5_280,
        tone: None,
    },
];

const NOTICE_INFO_ONCE: [Step; 9] = [
    Step {
        duration_ms: 80,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 80,
        tone: None,
    },
    Step {
        duration_ms: 80,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 80,
        tone: None,
    },
    Step {
        duration_ms: 80,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 80,
        tone: None,
    },
    Step {
        duration_ms: 80,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 80,
        tone: None,
    },
    Step {
        duration_ms: 1_460,
        tone: None,
    },
];

const NOTICE_WARN_ONCE: [Step; 5] = [
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 1_300,
        tone: None,
    },
];

const NOTICE_ERROR_ONCE: [Step; 13] = [
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: FREQ_NOTICE_HZ,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 200,
        tone: None,
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
];

const MELODY_MODE_READY: [Step; 16] = [
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_800,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_800,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 400,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_000,
        tone: None,
    },
];

const MELODY_MODE_CHARGE: [Step; 16] = [
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 280,
        tone: Some(Tone {
            freq_hz: 1_800,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 280,
        tone: Some(Tone {
            freq_hz: 1_800,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_500,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 420,
        tone: Some(Tone {
            freq_hz: 2_100,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_000,
        tone: None,
    },
];

const MELODY_MODE_DISCHARGE: [Step; 14] = [
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 2_100,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 180,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 180,
        tone: None,
    },
    Step {
        duration_ms: 280,
        tone: Some(Tone {
            freq_hz: 1_300,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 2_100,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 180,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 1_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 180,
        tone: None,
    },
    Step {
        duration_ms: 280,
        tone: Some(Tone {
            freq_hz: 1_300,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 500,
        tone: Some(Tone {
            freq_hz: 1_000,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_000,
        tone: None,
    },
];

const MELODY_MODE_LOWBATT: [Step; 28] = [
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_400,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_400,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_400,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_400,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 100,
        tone: None,
    },
    Step {
        duration_ms: 140,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 240,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 240,
        tone: None,
    },
    Step {
        duration_ms: 240,
        tone: Some(Tone {
            freq_hz: 1_000,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_000,
        tone: None,
    },
];

const MELODY_AC_LOST: [Step; 18] = [
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: 2_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: 2_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 2_000,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 2_300,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 220,
        tone: None,
    },
    Step {
        duration_ms: 220,
        tone: Some(Tone {
            freq_hz: 2_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: 2_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 200,
        tone: Some(Tone {
            freq_hz: 2_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 120,
        tone: None,
    },
    Step {
        duration_ms: 450,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 500,
        tone: None,
    },
    Step {
        duration_ms: 450,
        tone: Some(Tone {
            freq_hz: 1_300,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_200,
        tone: None,
    },
];

const MELODY_AC_RESTORED: [Step; 16] = [
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_000,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 260,
        tone: None,
    },
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_300,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 260,
        tone: None,
    },
    Step {
        duration_ms: 420,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 520,
        tone: None,
    },
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_100,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 260,
        tone: None,
    },
    Step {
        duration_ms: 300,
        tone: Some(Tone {
            freq_hz: 1_400,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 260,
        tone: None,
    },
    Step {
        duration_ms: 450,
        tone: Some(Tone {
            freq_hz: 1_700,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 650,
        tone: None,
    },
    Step {
        duration_ms: 450,
        tone: Some(Tone {
            freq_hz: 1_200,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 450,
        tone: None,
    },
    Step {
        duration_ms: 450,
        tone: Some(Tone {
            freq_hz: 1_600,
            duty_pct: DUTY_PCT_DEFAULT,
        }),
    },
    Step {
        duration_ms: 1_000,
        tone: None,
    },
];

fn pattern_for(sound: SoundId) -> Pattern {
    match sound {
        SoundId::ActionOk => Pattern {
            steps: &ACTION_OK,
            looped: false,
        },
        SoundId::ActionFail => Pattern {
            steps: &ACTION_FAIL,
            looped: false,
        },
        SoundId::ActionFault => Pattern {
            steps: &ACTION_FAULT,
            looped: false,
        },

        SoundId::MelodyModeReady => Pattern {
            steps: &MELODY_MODE_READY,
            looped: false,
        },
        SoundId::MelodyModeCharge => Pattern {
            steps: &MELODY_MODE_CHARGE,
            looped: false,
        },
        SoundId::MelodyModeDischarge => Pattern {
            steps: &MELODY_MODE_DISCHARGE,
            looped: false,
        },
        SoundId::MelodyModeLowbatt => Pattern {
            steps: &MELODY_MODE_LOWBATT,
            looped: false,
        },
        SoundId::MelodyAcLost => Pattern {
            steps: &MELODY_AC_LOST,
            looped: false,
        },
        SoundId::MelodyAcRestored => Pattern {
            steps: &MELODY_AC_RESTORED,
            looped: false,
        },

        SoundId::NoticeInfoOnce => Pattern {
            steps: &NOTICE_INFO_ONCE,
            looped: false,
        },
        SoundId::NoticeWarnOnce => Pattern {
            steps: &NOTICE_WARN_ONCE,
            looped: false,
        },
        SoundId::NoticeErrorOnce => Pattern {
            steps: &NOTICE_ERROR_ONCE,
            looped: false,
        },

        SoundId::AlarmLatchedLoop => Pattern {
            steps: &ALARM_LATCHED_LOOP,
            looped: true,
        },
        SoundId::AlarmThermalLoop => Pattern {
            steps: &ALARM_THERMAL_LOOP,
            looped: true,
        },
        SoundId::AlarmCommLoop => Pattern {
            steps: &ALARM_COMM_LOOP,
            looped: true,
        },
        SoundId::AlarmLowbattLoop => Pattern {
            steps: &ALARM_LOWBATT_LOOP,
            looped: true,
        },
    }
}

fn alarm_priority(sound: SoundId) -> Option<u8> {
    match sound {
        SoundId::AlarmLatchedLoop => Some(0),
        SoundId::AlarmThermalLoop => Some(1),
        SoundId::AlarmLowbattLoop => Some(2),
        SoundId::AlarmCommLoop => Some(3),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaybackKind {
    Background,
    Action,
}

#[derive(Clone, Copy)]
struct Playback {
    pattern: Pattern,
    index: usize,
    sound: SoundId,
    kind: PlaybackKind,
}

impl Playback {
    fn current_step(&self) -> Step {
        self.pattern.steps[self.index]
    }

    fn is_current_step_silence(&self) -> bool {
        self.current_step().tone.is_none()
    }

    fn advance(&mut self) -> bool {
        self.index += 1;
        if self.index < self.pattern.steps.len() {
            return true;
        }

        if self.pattern.looped {
            self.index = 0;
            return true;
        }

        false
    }
}

struct PromptToneManager {
    buzzer: Buzzer,
    active_alarm_mask: u8,
    current: Option<Playback>,
    paused: Option<Playback>,
    pending_action: Option<SoundId>,
    last_action_at: Option<Instant>,
    alarm_dirty: bool,
}

impl PromptToneManager {
    fn new(mut buzzer: Buzzer) -> Self {
        buzzer.stop();
        Self {
            buzzer,
            active_alarm_mask: 0,
            current: None,
            paused: None,
            pending_action: None,
            last_action_at: None,
            alarm_dirty: false,
        }
    }

    fn any_alarm_active(&self) -> bool {
        self.active_alarm_mask != 0
    }

    fn select_highest_alarm(&self) -> Option<SoundId> {
        let mut best: Option<(u8, SoundId)> = None;
        for &id in &[
            SoundId::AlarmLatchedLoop,
            SoundId::AlarmThermalLoop,
            SoundId::AlarmLowbattLoop,
            SoundId::AlarmCommLoop,
        ] {
            let Some(prio) = alarm_priority(id) else {
                continue;
            };
            let bit = 1u8 << prio;
            if (self.active_alarm_mask & bit) == 0 {
                continue;
            }
            best = Some((prio, id));
            break;
        }
        best.map(|(_, id)| id)
    }

    fn set_alarm_active(&mut self, alarm: SoundId, active: bool) {
        let Some(prio) = alarm_priority(alarm) else {
            return;
        };
        let bit = 1u8 << prio;
        if active {
            self.active_alarm_mask |= bit;
        } else {
            self.active_alarm_mask &= !bit;
        }
        self.alarm_dirty = true;
    }

    fn apply_step(&mut self, step: Step) {
        if let Some(tone) = step.tone {
            self.buzzer.start_tone(tone.freq_hz, Some(tone.duty_pct));
        } else {
            self.buzzer.stop();
        }
    }

    fn start_playback(&mut self, sound: SoundId, kind: PlaybackKind, clear_pause: bool) {
        let pattern = pattern_for(sound);
        self.current = Some(Playback {
            pattern,
            index: 0,
            sound,
            kind,
        });
        if clear_pause {
            self.paused = None;
            self.pending_action = None;
        }
        self.apply_step(self.current.unwrap().current_step());
    }

    fn maybe_start_action_now(&mut self, action: SoundId) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_action_at {
            if now.duration_since(last) < Duration::from_millis(ACTION_MIN_INTERVAL_MS) {
                return false;
            }
        }
        self.last_action_at = Some(now);

        let Some(current) = self.current else {
            self.start_playback(action, PlaybackKind::Action, true);
            return true;
        };

        if current.kind == PlaybackKind::Action {
            return false;
        }

        if current.is_current_step_silence() {
            self.paused = Some(current);
            self.start_playback(action, PlaybackKind::Action, false);
            return true;
        }

        self.pending_action = Some(action);
        false
    }

    fn ensure_alarm_playing(&mut self) {
        if !self.any_alarm_active() {
            return;
        }
        let Some(alarm) = self.select_highest_alarm() else {
            return;
        };
        if let Some(current) = self.current {
            if current.kind == PlaybackKind::Action {
                return;
            }
            if alarm_priority(current.sound).is_some() && current.sound == alarm && !self.alarm_dirty {
                return;
            }
        }
        self.start_playback(alarm, PlaybackKind::Background, true);
        self.alarm_dirty = false;
    }

    fn handle_request(&mut self, req: ToneRequest) {
        match req {
            ToneRequest::AlarmLoopEnter(id) => {
                self.set_alarm_active(id, true);
                self.ensure_alarm_playing();
            }
            ToneRequest::AlarmLoopExit(id) => {
                self.set_alarm_active(id, false);
                if !self.any_alarm_active() {
                    if let Some(current) = self.current {
                        if current.kind == PlaybackKind::Background
                            && alarm_priority(current.sound).is_some()
                        {
                            self.current = None;
                            self.paused = None;
                            self.pending_action = None;
                            self.buzzer.stop();
                        }
                    }
                } else if let Some(current) = self.current {
                    if current.kind == PlaybackKind::Background
                        && alarm_priority(current.sound).is_some()
                        && self.select_highest_alarm().is_some_and(|a| a != current.sound)
                    {
                        self.alarm_dirty = true;
                        if current.is_current_step_silence() {
                            self.ensure_alarm_playing();
                        }
                    }
                }
            }
            ToneRequest::Action(id) => {
                self.maybe_start_action_now(id);
            }
            ToneRequest::ModeMelody(id) => {
                if self.any_alarm_active() {
                    return;
                }
                self.start_playback(id, PlaybackKind::Background, true);
            }
            ToneRequest::NoticeOnce(id) => {
                if self.any_alarm_active() {
                    return;
                }
                self.start_playback(id, PlaybackKind::Background, true);
            }
        }
    }

    fn on_step_boundary(&mut self) {
        if self.any_alarm_active() && self.alarm_dirty {
            if let Some(current) = self.current {
                if current.kind == PlaybackKind::Background && alarm_priority(current.sound).is_some() {
                    self.ensure_alarm_playing();
                    return;
                }
            }
        }

        if self.current.is_none() {
            self.buzzer.stop();
            return;
        }

        let mut current = self.current.unwrap();
        if !current.advance() {
            // Playback ended.
            match current.kind {
                PlaybackKind::Action => {
                    if let Some(paused) = self.paused.take() {
                        self.current = Some(paused);
                    } else {
                        self.current = None;
                    }
                }
                PlaybackKind::Background => {
                    self.current = None;
                }
            }
            self.buzzer.stop();
            if self.any_alarm_active() {
                self.ensure_alarm_playing();
            }
            if let Some(cur) = self.current {
                self.apply_step(cur.current_step());
            }
            return;
        } else {
            self.current = Some(current);
        }

        if let Some(action) = self.pending_action.take() {
            if let Some(cur) = self.current {
                if cur.kind == PlaybackKind::Background && cur.is_current_step_silence() {
                    self.paused = Some(cur);
                    self.start_playback(action, PlaybackKind::Action, false);
                    return;
                }
            }
        }

        if let Some(cur) = self.current {
            self.apply_step(cur.current_step());
        }

        if self.any_alarm_active() {
            self.ensure_alarm_playing();
        }
    }
}

#[task]
pub async fn tone_task(buzzer: Buzzer, mut rx: ToneRequestReceiver) {
    let mut mgr = PromptToneManager::new(buzzer);
    mgr.buzzer.stop();

    loop {
        if mgr.current.is_none() {
            // Idle: wait for something to do.
            let req = rx.recv().await;
            mgr.handle_request(req);
            if mgr.any_alarm_active() {
                mgr.ensure_alarm_playing();
            }
            continue;
        }

        let Some(cur) = mgr.current else {
            continue;
        };
        let wait_ms = cur.current_step().duration_ms as u64;
        let wait_fut = Timer::after(Duration::from_millis(wait_ms));
        match select(rx.recv(), wait_fut).await {
            Either::First(req) => {
                mgr.handle_request(req);
                if mgr.any_alarm_active() {
                    mgr.ensure_alarm_playing();
                }
            }
            Either::Second(_) => {
                mgr.on_step_boundary();
            }
        }
    }
}
