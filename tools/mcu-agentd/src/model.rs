use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McuKind {
    Esp32,
    Stm32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfterPolicy {
    NoReset,
    HardReset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientRequest {
    Shutdown,
    Status,
    SetPort {
        mcu: McuKind,
        path: PathBuf,
    },
    GetPort {
        mcu: McuKind,
    },
    StartMonitor {
        mcu: McuKind,
    },
    StopMonitor {
        mcu: McuKind,
    },
    Flash {
        mcu: McuKind,
        elf: PathBuf,
        after: Option<AfterPolicy>,
    },
    Reset {
        mcu: McuKind,
    },
    Monitor {
        mcu: McuKind,
        elf: Option<PathBuf>,
        duration: Option<u64>, // milliseconds
        lines: Option<usize>,
    },
    Logs {
        mcu: Option<McuKind>,
        since: Option<String>,
        until: Option<String>,
        tail: Option<usize>,
        /// include session lines (tail per session)
        sessions: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientResponse {
    pub ok: bool,
    pub message: Option<String>,
    pub payload: serde_json::Value,
}

impl ClientResponse {
    pub fn ok<T: Serialize>(payload: T) -> Self {
        Self {
            ok: true,
            message: None,
            payload: serde_json::to_value(payload).unwrap_or_default(),
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(msg.into()),
            payload: serde_json::Value::Null,
        }
    }
}
