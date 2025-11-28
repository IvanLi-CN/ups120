mod model;
mod paths;
mod port_cache;
mod process;
mod server;
mod timefmt;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::Select;
use dialoguer::theme::ColorfulTheme;
use model::{ClientRequest, McuKind};
use serialport::{SerialPortType, available_ports};
use server::Server;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;
use tokio::time::Duration as TokioDuration;
use tokio::time::sleep;

/// UPS120 agentd – single-instance service to flash/reset/monitor ESP32-S3 & STM32L0.
#[derive(Parser, Debug)]
#[command(name = "ups120-agentd", version)]
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
        /// Leave empty to interactively select (ESP32 only)
        path: Option<PathBuf>,
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
        Cmd::Status => {
            // Try to give a more actionable error message instead of a generic "not running".
            let paths = paths::Paths::new().ok();
            match Server::client_send(ClientRequest::Status).await {
                Ok(resp) => println!("{}", serde_json::to_string_pretty(&resp)?),
                Err(e) => {
                    if let Some(ioe) = e.downcast_ref::<std::io::Error>() {
                        use std::io::ErrorKind::*;
                        if let Some(p) = paths {
                            let sock = &p.sock;
                            match ioe.kind() {
                                NotFound => {
                                    eprintln!(
                                        "status: agentd socket {:?} not found: {}.\n  \
hint: daemon未在运行，或者 logs/agentd 被手动清理导致 sock 丢失；通常可用 `just agentd-start` 重新启动。\n  \
若你在 daemon 运行时删除了该目录，可能还残留旧的 ups120-agentd 进程，需要先杀掉进程再重启。",
                                        sock, ioe
                                    );
                                }
                                ConnectionRefused | BrokenPipe | ConnectionReset => {
                                    eprintln!(
                                        "status: 无法连接到 agentd {:?}: {} (连接被拒绝/中断)。\n  \
hint: daemon 正在启动、已崩溃或刚退出，可尝试 `just agentd-start` 重启。",
                                        sock, ioe
                                    );
                                }
                                PermissionDenied => {
                                    eprintln!(
                                        "status: 访问 agentd socket {:?} 权限不足: {}。\n  \
hint: 检查 logs/agentd 目录以及 sock 文件的所有者与权限。",
                                        sock, ioe
                                    );
                                }
                                _ => {
                                    eprintln!(
                                        "status: 查询 agentd 状态失败 (socket {:?}): {:#}",
                                        sock, e
                                    );
                                }
                            }
                        } else {
                            eprintln!("status: 查询 agentd 状态失败: {:#}", e);
                        }
                    } else {
                        eprintln!("status: 查询 agentd 状态失败: {:#}", e);
                    }
                }
            }
        }
        Cmd::SetPort { mcu, path } => {
            let mcu_kind: McuKind = mcu.clone().into();
            let p = match path {
                Some(p) => p,
                None => interactive_select_port(mcu_kind.clone()).await?,
            };
            let resp = client_send_with_autostart(ClientRequest::SetPort {
                mcu: mcu_kind,
                path: p,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::GetPort { mcu } => {
            let resp =
                client_send_with_autostart(ClientRequest::GetPort { mcu: mcu.into() }).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Flash { mcu, elf, after } => {
            let resp = client_send_with_autostart(ClientRequest::Flash {
                mcu: mcu.into(),
                elf,
                after: Some(after.into()),
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Cmd::Reset { mcu } => {
            let resp = client_send_with_autostart(ClientRequest::Reset { mcu: mcu.into() }).await?;
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
            let resp = client_send_with_autostart(ClientRequest::Logs {
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

// Try sending to daemon; if socket不存在则自动启动并重试一次。
async fn client_send_with_autostart(req: ClientRequest) -> Result<model::ClientResponse> {
    let paths = paths::Paths::new()?;
    let sock = paths.sock.clone();
    match Server::client_send(req.clone()).await {
        Ok(r) => Ok(r),
        Err(e) => {
            let (is_enoent, is_refused) = match e.downcast_ref::<std::io::Error>() {
                Some(ioe) => (
                    ioe.kind() == std::io::ErrorKind::NotFound,
                    matches!(
                        ioe.kind(),
                        std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionReset
                    ),
                ),
                None => (false, false),
            };
            if !(is_enoent || is_refused) {
                return Err(e);
            }
            // auto-start daemon then retry once
            Server::spawn_background().await?;
            // wait a bit for socket ready
            sleep(TokioDuration::from_millis(150)).await;
            let mut attempts = 0;
            loop {
                attempts += 1;
                match Server::client_send(req.clone()).await {
                    Ok(r) => return Ok(r),
                    Err(_e) if attempts < 5 => {
                        sleep(TokioDuration::from_millis(150)).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "agentd not reachable at {:?}: {}. Try `just agentd-start` or check permissions (logs/agentd).",
                            sock,
                            e
                        ));
                    }
                }
            }
        }
    }
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

    // track current offset; start at end (tail-only)
    let mut pos = std::fs::metadata(&session)?.len();
    let mut chunk = String::new();

    if std::env::var_os("MON_DEBUG").is_some() {
        eprintln!("session {:?} start pos={} bytes", session, pos);
    }

    let deadline = if duration.as_millis() == 0 {
        None
    } else {
        Some(Instant::now() + duration)
    };
    let mut printed = 0usize;

    loop {
        if std::env::var_os("MON_DEBUG").is_some() {
            eprintln!("loop start pos={}", pos);
        }
        let len = std::fs::metadata(&session)?.len();
        if len <= pos {
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            continue;
        }

        if std::env::var_os("MON_DEBUG").is_some() {
            eprintln!("monitor tick len={} pos={}", len, pos);
        }

        let mut file = std::fs::OpenOptions::new().read(true).open(&session)?;
        file.seek(SeekFrom::Start(pos))?;
        chunk.clear();
        file.read_to_string(&mut chunk)?;
        pos = len;

        for line in chunk.lines() {
            let line = line.trim_end_matches(['\n', '\r'].as_ref());
            // try extract text field if JSON line
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(txt) = val.get("text").and_then(|t| t.as_str()) {
                    println!("{}", colorize_line(txt));
                } else {
                    println!("{}", colorize_line(line));
                }
            } else {
                println!("{}", colorize_line(line));
            }
            printed += 1;
            if lines > 0 && printed >= lines {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn colorize_line(line: &str) -> String {
    // passthrough: keep original text (and any ANSI sequences) untouched
    line.to_string()
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

async fn interactive_select_port(mcu: McuKind) -> Result<PathBuf> {
    match mcu {
        McuKind::Esp32 => {
            let ports = available_ports()?;
            if ports.is_empty() {
                bail!("未发现串口，请先接好设备再试");
            }
            // espflash-aligned, macOS friendly: only Espressif USB (VID=0x303A) and prefer cu.* device nodes
            let filtered: Vec<&serialport::SerialPortInfo> = ports
                .iter()
                .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(ref info) if info.vid == 0x303A))
                .filter(|p| p.port_name.contains("/cu."))
                .collect();

            if filtered.is_empty() {
                bail!("未找到 Espressif USB 串口（仅列 cu.*，VID=303A），可用 --path 显式指定");
            }
            let items: Vec<String> = filtered
                .iter()
                .map(|p| {
                    let extra = match &p.port_type {
                        SerialPortType::UsbPort(info) => format!(
                            "vid={:04x} pid={:04x} {:?}",
                            info.vid, info.pid, info.product
                        ),
                        _ => String::new(),
                    };
                    format!("{} ({extra})", p.port_name)
                })
                .collect();
            let idx = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("选择 ESP32 串口 (上下箭头选择，回车确认)")
                .items(&items)
                .default(0)
                .interact()?;
            Ok(PathBuf::from(filtered[idx].port_name.clone()))
        }
        McuKind::Stm32 => {
            use tokio::process::Command;
            let out = Command::new("probe-rs").arg("list").output().await?;
            if !out.status.success() {
                bail!(
                    "probe-rs list 失败: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut entries: Vec<String> = stdout
                .lines()
                .filter(|l| l.trim_start().starts_with('['))
                .map(|l| l.trim().to_string())
                .collect();
            // Prefer STM32-friendly probes (STLink / CMSIS-DAP), drop ESP JTAG/WCH when possible
            let preferred: Vec<String> = entries
                .iter()
                .filter(|l| {
                    l.contains("STLink")
                        || l.contains("ST-LINK")
                        || l.contains("CMSIS-DAP")
                        || l.contains("0483:3748")
                        || l.contains("0d28:0204")
                })
                .cloned()
                .collect();
            if !preferred.is_empty() {
                entries = preferred;
            }

            if entries.is_empty() {
                bail!("未发现调试探针，可用 --path 显式指定 probe-rs 标识");
            }
            let idx = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("选择 STM32 调试探针 (上下箭头选择，回车确认)")
                .items(&entries)
                .default(0)
                .interact()?;
            let selected = &entries[idx];
            // line format: [1]: STLink V2 -- 0483:3748:SERIAL (ST-LINK)
            let id = selected
                .split("--")
                .nth(1)
                .map(|s| s.trim().split_whitespace().next().unwrap_or(s.trim()))
                .unwrap_or(selected);
            Ok(PathBuf::from(id))
        }
    }
}
