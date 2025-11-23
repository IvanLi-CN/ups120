mod model;
mod paths;
mod port_cache;
mod process;
mod server;
mod timefmt;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use model::{ClientRequest, McuKind};
use server::Server;
use std::path::PathBuf;

/// MCU agentd – single-instance service to flash/reset/monitor ESP32-S3 & STM32L0.
#[derive(Parser, Debug)]
#[command(name = "mcu-agentd", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Start daemon in background (if not running).
    Start,
    /// Stop daemon if running.
    Stop,
    /// Query daemon status.
    Status,
    /// Set cached port/probe selector for MCU.
    SetPort {
        #[arg(value_enum)]
        mcu: McuOpt,
        path: PathBuf,
    },
    /// Get cached port/probe selector for MCU.
    GetPort {
        #[arg(value_enum)]
        mcu: McuOpt,
    },
    /// Flash firmware to MCU (does not auto-run).
    Flash {
        #[arg(value_enum)]
        mcu: McuOpt,
        /// ELF path
        elf: PathBuf,
        /// esp32 after-reset policy
        #[arg(long, default_value = "no-reset", value_enum)]
        after: OptionAfter,
    },
    /// Reset MCU.
    Reset {
        #[arg(value_enum)]
        mcu: McuOpt,
    },
    /// Stop background monitor for MCU.
    StopMon {
        #[arg(value_enum)]
        mcu: McuOpt,
    },
    /// Start background monitor for MCU.
    StartMon {
        #[arg(value_enum)]
        mcu: McuOpt,
    },
    /// Monitor MCU logs (attach/monitor without auto-flash unless required).
    Monitor {
        #[arg(value_enum)]
        mcu: McuOpt,
        /// Optional ELF path; if省略则尝试默认构建产物。
        elf: Option<PathBuf>,
        /// Auto-stop after duration, e.g. 30s/2m/1h (0 = unlimited).
        #[arg(long, value_parser = humantime::parse_duration, default_value = "0")]
        duration: std::time::Duration,
        /// Auto-stop after N lines (0 = unlimited).
        #[arg(long, default_value = "0")]
        lines: usize,
    },
    /// Fetch logs (server-side filtered).
    Logs {
        #[arg(value_enum)]
        mcu: LogsMcuOpt,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        tail: Option<usize>,
        #[arg(
            long,
            default_value_t = false,
            help = "include session log lines (tail per session)"
        )]
        sessions: bool,
    },
    /// Internal: run daemon foreground (do not call directly).
    #[command(hide = true)]
    Serve,
}

#[derive(Clone, Debug, ValueEnum)]
enum McuOpt {
    Esp32,
    Stm32,
}

#[derive(Clone, Debug, ValueEnum)]
enum LogsMcuOpt {
    Esp32,
    Stm32,
    All,
}

#[derive(Clone, Debug, ValueEnum)]
enum OptionAfter {
    #[value(name = "no-reset")]
    NoReset,
    #[value(name = "hard-reset")]
    HardReset,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve => {
            Server::run().await?;
        }
        Cmd::Start => {
            Server::spawn_background().await?;
            println!("ok");
        }
        Cmd::Stop => {
            let resp = Server::client_send(ClientRequest::Shutdown).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Status => match Server::client_send(ClientRequest::Status).await {
            Ok(resp) => println!("{}", serde_json::to_string_pretty(&resp)?),
            Err(e) => {
                eprintln!("status: not running ({e})");
            }
        },
        Cmd::SetPort { mcu, path } => {
            let resp = Server::client_send(ClientRequest::SetPort {
                mcu: mcu.into(),
                path,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::GetPort { mcu } => {
            let resp = Server::client_send(ClientRequest::GetPort { mcu: mcu.into() }).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Flash { mcu, elf, after } => {
            let resp = Server::client_send(ClientRequest::Flash {
                mcu: mcu.into(),
                elf,
                after: Some(after.into()),
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Reset { mcu } => {
            let resp = Server::client_send(ClientRequest::Reset { mcu: mcu.into() }).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::StopMon { mcu } => {
            let resp = Server::client_send(ClientRequest::StopMonitor { mcu: mcu.into() }).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::StartMon { mcu } => {
            let resp = Server::client_send(ClientRequest::StartMonitor { mcu: mcu.into() }).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Monitor {
            mcu,
            elf,
            duration,
            lines,
        } => {
            let resp = Server::client_send(ClientRequest::Monitor {
                mcu: mcu.into(),
                elf,
                duration: if duration.as_millis() == 0 {
                    None
                } else {
                    Some(duration.as_millis() as u64)
                },
                lines: if lines == 0 { None } else { Some(lines) },
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Logs {
            mcu,
            since,
            until,
            tail,
            sessions,
        } => {
            let resp = Server::client_send(ClientRequest::Logs {
                mcu: mcu.into(),
                since,
                until,
                tail,
                sessions,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
    }
    Ok(())
}

impl From<McuOpt> for McuKind {
    fn from(m: McuOpt) -> Self {
        match m {
            McuOpt::Esp32 => McuKind::Esp32,
            McuOpt::Stm32 => McuKind::Stm32,
        }
    }
}

impl From<LogsMcuOpt> for Option<McuKind> {
    fn from(m: LogsMcuOpt) -> Self {
        match m {
            LogsMcuOpt::Esp32 => Some(McuKind::Esp32),
            LogsMcuOpt::Stm32 => Some(McuKind::Stm32),
            LogsMcuOpt::All => None,
        }
    }
}

impl From<OptionAfter> for model::AfterPolicy {
    fn from(a: OptionAfter) -> Self {
        match a {
            OptionAfter::NoReset => model::AfterPolicy::NoReset,
            OptionAfter::HardReset => model::AfterPolicy::HardReset,
        }
    }
}
