use crate::model::{AfterPolicy, ClientRequest, ClientResponse, McuKind};
use crate::paths::Paths;
use crate::port_cache;
use crate::process::run_mcu_cmd;
use crate::timefmt::{Clock, Timestamp};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBuf};
use tokio::net::UnixListener;
use tokio::net::UnixStream;
use tokio::process::Command;

pub struct Server;

impl Server {
    pub async fn run() -> Result<()> {
        let paths = Paths::new()?;
        paths.ensure_dirs()?;
        if paths.sock.exists() {
            let _ = std::fs::remove_file(&paths.sock);
        }
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(paths.lock_path())?;
        lock_file
            .try_lock_exclusive()
            .context("another instance running")?;

        let clock = Clock::new();
        let listener = UnixListener::bind(&paths.sock)?;
        println!("mcu-agentd listening at {:?}", paths.sock);
        loop {
            let (stream, _) = listener.accept().await?;
            let paths_cl = paths.clone();
            let clock_cl = clock;
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, paths_cl, clock_cl).await {
                    eprintln!("conn error: {e:?}");
                }
            });
        }
    }

    pub async fn spawn_background() -> Result<()> {
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null());
        cmd.spawn().context("spawn daemon")?;
        Ok(())
    }

    pub async fn client_send(req: ClientRequest) -> Result<ClientResponse> {
        let paths = Paths::new()?;
        let stream = UnixStream::connect(&paths.sock).await?;
        let mut stream = stream;
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        stream.write_all(line.as_bytes()).await?;
        let mut reader = TokioBuf::new(stream);
        let mut resp_line = String::new();
        reader.read_line(&mut resp_line).await?;
        let resp: ClientResponse = serde_json::from_str(&resp_line)?;
        Ok(resp)
    }
}

async fn handle_conn(stream: UnixStream, paths: Paths, clock: Clock) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = TokioBuf::new(read_half);
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    let req: ClientRequest = serde_json::from_str(&buf)?;
    let resp = handle_request(req, &paths, &clock)
        .await
        .unwrap_or_else(|e| ClientResponse::err(format!("{e:#}")));
    let line = serde_json::to_string(&resp)? + "\n";
    write_half.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn handle_request(
    req: ClientRequest,
    paths: &Paths,
    clock: &Clock,
) -> Result<ClientResponse> {
    match req {
        ClientRequest::Shutdown => {
            std::fs::remove_file(&paths.sock).ok();
            Ok(ClientResponse::ok(json!({"status":"stopping"})))
        }
        ClientRequest::Status => {
            let ts = clock.now();
            Ok(ClientResponse::ok(json!({
                "ts": ts.iso(),
                "pid": std::process::id(),
                "sock": paths.sock,
            })))
        }
        ClientRequest::SetPort { mcu, path } => {
            port_cache::write_port(paths, mcu.clone(), path.to_string_lossy().as_ref())?;
            let ts = clock.now();
            Ok(ClientResponse::ok(
                json!({"ts": ts.iso(), "mcu": mcu, "path": path}),
            ))
        }
        ClientRequest::GetPort { mcu } => {
            let ts = clock.now();
            let val = port_cache::read_port(paths, mcu.clone())?;
            Ok(ClientResponse::ok(
                json!({"ts": ts.iso(), "mcu": mcu, "path": val}),
            ))
        }
        ClientRequest::Flash { mcu, elf, after } => {
            let ts = clock.now();
            let res = flash_mcu(
                paths,
                &mcu,
                &elf,
                after.unwrap_or(AfterPolicy::NoReset),
                &ts,
            )
            .await?;
            Ok(ClientResponse::ok(res))
        }
        ClientRequest::Reset { mcu } => {
            let ts = clock.now();
            let res = reset_mcu(paths, &mcu, &ts).await?;
            Ok(ClientResponse::ok(res))
        }
        ClientRequest::Logs {
            mcu,
            since,
            until,
            tail,
        } => {
            let entries = query_logs(paths, mcu, since, until, tail)?;
            Ok(ClientResponse::ok(entries))
        }
    }
}

async fn flash_mcu(
    paths: &Paths,
    mcu: &McuKind,
    elf: &PathBuf,
    after: AfterPolicy,
    ts: &Timestamp,
) -> Result<serde_json::Value> {
    let cmd = match mcu {
        McuKind::Esp32 => {
            let port = require_port(paths, McuKind::Esp32)?;
            let mut c = Command::new("espflash");
            c.arg("flash")
                .arg(elf)
                .arg("--chip")
                .arg("esp32s3")
                .arg("--port")
                .arg(port)
                .arg("--after")
                .arg(match after {
                    AfterPolicy::NoReset => "no-reset",
                    AfterPolicy::HardReset => "hard-reset",
                });
            c
        }
        McuKind::Stm32 => {
            let probe = require_port(paths, McuKind::Stm32)?;
            let mut c = Command::new("probe-rs");
            c.arg("download")
                .arg("--chip")
                .arg("STM32L051C8Tx")
                .arg("--probe")
                .arg(probe)
                .arg(elf);
            c
        }
    };
    let res = run_mcu_cmd(paths, mcu, ts, cmd).await?;
    write_meta(paths, mcu, ts, "flash", &res)?;
    Ok(json!({
        "ts": ts.iso(),
        "mcu": mcu,
        "status": res.status,
        "duration_ms": res.duration_ms,
        "session": res.session_file,
    }))
}

async fn reset_mcu(paths: &Paths, mcu: &McuKind, ts: &Timestamp) -> Result<serde_json::Value> {
    let cmd = match mcu {
        McuKind::Esp32 => {
            let port = require_port(paths, McuKind::Esp32)?;
            let mut c = Command::new("espflash");
            c.arg("reset")
                .arg("--chip")
                .arg("esp32s3")
                .arg("--port")
                .arg(port);
            c
        }
        McuKind::Stm32 => {
            let probe = require_port(paths, McuKind::Stm32)?;
            let mut c = Command::new("probe-rs");
            c.arg("reset")
                .arg("--chip")
                .arg("STM32L051C8Tx")
                .arg("--probe")
                .arg(probe);
            c
        }
    };
    let res = run_mcu_cmd(paths, mcu, ts, cmd).await?;
    write_meta(paths, mcu, ts, "reset", &res)?;
    Ok(json!({
        "ts": ts.iso(),
        "mcu": mcu,
        "status": res.status,
        "duration_ms": res.duration_ms,
        "session": res.session_file,
    }))
}

fn write_meta(
    paths: &Paths,
    mcu: &McuKind,
    ts: &Timestamp,
    event: &str,
    res: &crate::process::RunResult,
) -> Result<()> {
    let meta_path = paths.meta(mcu.clone());
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(meta_path)?;
    let line = json!({
        "ts": ts.iso(),
        "mono_ms": ts.mono_ms(),
        "mcu": match mcu { McuKind::Esp32 => "esp32", McuKind::Stm32 => "stm32" },
        "event": event,
        "status": res.status,
        "duration_ms": res.duration_ms,
        "session": res.session_file,
    });
    writeln!(f, "{}", line)?;
    Ok(())
}

fn query_logs(
    paths: &Paths,
    mcu: Option<McuKind>,
    since: Option<String>,
    until: Option<String>,
    tail: Option<usize>,
) -> Result<serde_json::Value> {
    let files: Vec<PathBuf> = match mcu {
        Some(McuKind::Esp32) => vec![paths.meta_esp32.clone()],
        Some(McuKind::Stm32) => vec![paths.meta_stm32.clone()],
        None => vec![paths.meta_esp32.clone(), paths.meta_stm32.clone()],
    };
    let mut rows = Vec::new();
    for f in files {
        if !f.exists() {
            continue;
        }
        let reader = BufReader::new(File::open(&f)?);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(&line)?;
            if !passes_time(&v, since.as_deref(), until.as_deref()) {
                continue;
            }
            rows.push(v);
        }
    }
    if let Some(n) = tail {
        if rows.len() > n {
            rows = rows.split_off(rows.len() - n);
        }
    }
    Ok(json!(rows))
}

fn passes_time(v: &serde_json::Value, since: Option<&str>, until: Option<&str>) -> bool {
    let ts = v.get("ts").and_then(|t| t.as_str()).unwrap_or("");
    if let Some(s) = since {
        if ts < s {
            return false;
        }
    }
    if let Some(u) = until {
        if ts > u {
            return false;
        }
    }
    true
}

fn require_port(paths: &Paths, mcu: McuKind) -> Result<String> {
    port_cache::read_port(paths, mcu.clone())?
        .ok_or_else(|| anyhow::anyhow!("port/probe cache missing for {:?}", mcu))
}
