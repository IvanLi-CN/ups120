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
use std::time::Instant;
use tokio::fs::OpenOptions as TokioOpen;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

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
        Cmd::Monitor {
            mcu,
            duration,
            lines,
            ..
        } => {
            monitor_tail(mcu.into(), duration, lines).await?;
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

async fn monitor_tail(mcu: McuKind, duration: std::time::Duration, lines: usize) -> Result<()> {
    let paths = paths::Paths::new()?;
    let dir = paths.session_dir(mcu.clone());
    let session =
        latest_session(dir)?.ok_or_else(|| anyhow::anyhow!("no session log for {:?}", mcu))?;

    let mut file = TokioOpen::new().read(true).open(&session).await?;
    // tail-only: start at end
    file.seek(std::io::SeekFrom::End(0)).await?;
    let mut reader = BufReader::new(file);

    let deadline = if duration.as_millis() == 0 {
        None
    } else {
        Some(Instant::now() + duration)
    };
    let mut printed = 0usize;

    loop {
        let mut buf = String::new();
        let n = tokio::select! {
            res = reader.read_line(&mut buf) => res?,
            _ = async { if let Some(dl)=deadline { tokio::time::sleep_until(dl.into()).await; } }, if deadline.is_some() => 0,
        };
        if n == 0 {
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            continue;
        }
        let line = buf.trim_end();
        // try extract text field if JSON line
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(txt) = val.get("text").and_then(|t| t.as_str()) {
                println!("{}", txt);
            } else {
                println!("{}", line);
            }
        } else {
            println!("{}", line);
        }
        printed += 1;
        if lines > 0 && printed >= lines {
            break;
        }
    }
    Ok(())
}

fn latest_session(dir: &std::path::Path) -> Result<Option<std::path::PathBuf>> {
    if !dir.exists() {
        return Ok(None);
    }
    let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        let mt = entry.metadata()?.modified()?;
        if latest.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
            latest = Some((mt, path));
        }
    }
    Ok(latest.map(|(_, p)| p))
}
