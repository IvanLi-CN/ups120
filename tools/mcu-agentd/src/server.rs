use crate::model::{AfterPolicy, ClientRequest, ClientResponse, McuKind};
use crate::paths::Paths;
use crate::port_cache;
use crate::process::run_mcu_cmd;
use crate::timefmt::{Clock, Timestamp};
use anyhow::{Context, Result};
use chrono::Local;
use fs2::FileExt;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader as TokioBuf};
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
        let running = Arc::new(AtomicBool::new(true));

        // auto-start monitors if ports + ELF 存在（不触发构建）
        let auto_paths = paths.clone();
        let auto_clock = clock;
        let auto_running = running.clone();
        tokio::spawn(async move {
            if let Err(e) = autostart_monitors(auto_paths, auto_clock, auto_running).await {
                eprintln!("autostart monitor error: {e:?}");
            }
        });

        while running.load(Ordering::SeqCst) {
            let (stream, _) = listener.accept().await?;
            let paths_cl = paths.clone();
            let clock_cl = clock;
            let running_cl = running.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, paths_cl, clock_cl, running_cl).await {
                    eprintln!("conn error: {e:?}");
                }
            });
        }
        Ok(())
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

async fn handle_conn(
    stream: UnixStream,
    paths: Paths,
    clock: Clock,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = TokioBuf::new(read_half);
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    let req: ClientRequest = serde_json::from_str(&buf)?;
    let resp = handle_request(req, &paths, &clock, &running)
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
    running: &Arc<AtomicBool>,
) -> Result<ClientResponse> {
    match req {
        ClientRequest::Shutdown => {
            std::fs::remove_file(&paths.sock).ok();
            running.store(false, Ordering::SeqCst);
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
        ClientRequest::StartAutoMonitor { mcu } => {
            let ts = clock.now();
            let res = start_auto_monitor(paths, &mcu, Some(ts)).await?;
            Ok(ClientResponse::ok(res))
        }
        ClientRequest::StopAutoMonitor { mcu } => {
            stop_auto_monitor(paths, &mcu)?;
            Ok(ClientResponse::ok(json!({"stopped": mcu})))
        }
        ClientRequest::Monitor {
            mcu,
            elf: _,
            duration,
            lines,
        } => {
            let res = tail_session(paths, &mcu, duration, lines).await?;
            Ok(ClientResponse::ok(res))
        }
        ClientRequest::Logs {
            mcu,
            since,
            until,
            tail,
            sessions,
        } => {
            let entries = query_logs(paths, mcu, since.clone(), until.clone(), tail)?;
            let sessions_payload = if sessions {
                query_session_logs(paths, &entries, since, until, tail)?
            } else {
                json!([])
            };
            Ok(ClientResponse::ok(
                json!({"meta": entries, "sessions": sessions_payload}),
            ))
        }
    }
}

async fn autostart_monitors(paths: Paths, clock: Clock, running: Arc<AtomicBool>) -> Result<()> {
    // ESP32
    if port_cache::read_port(&paths, McuKind::Esp32)?.is_some() {
        let ts = clock.now();
        if let Err(e) = start_auto_monitor(&paths, &McuKind::Esp32, Some(ts)).await {
            eprintln!("auto monitor esp32 failed: {e:#}");
        }
    }
    // STM32
    if port_cache::read_port(&paths, McuKind::Stm32)?.is_some() {
        let ts = clock.now();
        if let Err(e) = start_auto_monitor(&paths, &McuKind::Stm32, Some(ts)).await {
            eprintln!("auto monitor stm32 failed: {e:#}");
        }
    }
    // keep running flag so main loop continues; autostart returns immediately
    let _ = running;
    Ok(())
}

async fn start_auto_monitor(
    paths: &Paths,
    mcu: &McuKind,
    ts_opt: Option<Timestamp>,
) -> Result<serde_json::Value> {
    // stop any existing monitor for same MCU to free port
    stop_auto_monitor(paths, mcu)?;

    let ts = ts_opt.unwrap_or_else(|| Timestamp {
        wall: Local::now(),
        mono_ms: Duration::from_millis(0),
    });
    let elf = ensure_elf(paths, mcu, None).await?;
    let session = session_path_now(paths, mcu, &ts, true);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session)?;
    let file_err = file.try_clone()?;

    let mut cmd = match mcu {
        McuKind::Esp32 => {
            let port = require_port(paths, McuKind::Esp32)?;
            let mut c = Command::new("espflash");
            c.arg("monitor")
                .arg("--chip")
                .arg("esp32s3")
                .arg("--port")
                .arg(port)
                .arg("--non-interactive")
                .arg("--no-reset")
                .arg("--elf")
                .arg(&elf)
                .arg("--log-format")
                .arg("defmt");
            c
        }
        McuKind::Stm32 => {
            let probe = require_port(paths, McuKind::Stm32)?;
            let mut c = Command::new("probe-rs");
            c.arg("attach")
                .arg("--chip")
                .arg("STM32L051C8Tx")
                .arg("--probe")
                .arg(probe)
                .arg("--log-format")
                .arg("oneline")
                .arg(&elf);
            c
        }
    };
    cmd.stdout(std::process::Stdio::from(file));
    cmd.stderr(std::process::Stdio::from(file_err));

    let mut child = cmd.spawn().context("spawn auto monitor")?;
    let pid = child.id().unwrap_or(0);
    std::fs::write(paths.auto_pid(mcu.clone()), pid.to_string())?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    write_meta(
        paths,
        mcu,
        &ts,
        "auto-monitor-start",
        &crate::process::RunResult {
            status: 0,
            duration_ms: 0,
            session_file: session.clone(),
        },
    )?;

    Ok(json!({
        "pid": pid,
        "session": session,
        "elf": elf,
    }))
}

fn stop_auto_monitor(paths: &Paths, mcu: &McuKind) -> Result<()> {
    let pid_file = paths.auto_pid(mcu.clone());
    if let Ok(pid_txt) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid_num) = pid_txt.trim().parse::<i32>() {
            let _ = kill(Pid::from_raw(pid_num), Signal::SIGTERM);
        }
    }
    let _ = std::fs::remove_file(pid_file);
    Ok(())
}

async fn flash_mcu(
    paths: &Paths,
    mcu: &McuKind,
    elf: &PathBuf,
    after: AfterPolicy,
    ts: &Timestamp,
) -> Result<serde_json::Value> {
    // free port from auto monitor first
    stop_auto_monitor(paths, mcu)?;

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

    // restart auto monitor
    let _ = start_auto_monitor(paths, mcu, None).await;

    Ok(json!({
        "ts": ts.iso(),
        "mcu": mcu,
        "status": res.status,
        "duration_ms": res.duration_ms,
        "session": res.session_file,
    }))
}

async fn reset_mcu(paths: &Paths, mcu: &McuKind, ts: &Timestamp) -> Result<serde_json::Value> {
    stop_auto_monitor(paths, mcu)?;

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

    let _ = start_auto_monitor(paths, mcu, None).await;

    Ok(json!({
        "ts": ts.iso(),
        "mcu": mcu,
        "status": res.status,
        "duration_ms": res.duration_ms,
        "session": res.session_file,
    }))
}

async fn ensure_elf(paths: &Paths, mcu: &McuKind, elf: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = elf {
        if p.exists() {
            return Ok(p);
        }
        anyhow::bail!("ELF not found: {:?}", p);
    }
    match mcu {
        McuKind::Esp32 => {
            default_elf(
                paths,
                "ups-build",
                "firmware/ups-main/target/xtensa-esp32s3-none-elf/release/ups-main",
                true,
            )
            .await
        }
        McuKind::Stm32 => {
            default_elf(
                paths,
                "sb-build",
                "target/thumbv6m-none-eabi/release/smart-battery",
                true,
            )
            .await
        }
    }
}

async fn default_elf(
    paths: &Paths,
    make_target: &str,
    rel_path: &str,
    build: bool,
) -> Result<PathBuf> {
    let p = paths.root().join(rel_path);
    if p.exists() {
        return Ok(p);
    }
    if build {
        let status = Command::new("make")
            .arg(make_target)
            .current_dir(paths.root())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("make {make_target} failed: {status}");
        }
        if p.exists() {
            return Ok(p);
        }
        anyhow::bail!("built ELF missing after {make_target}: {:?}", p)
    }
    anyhow::bail!("default ELF missing and build disabled: {:?}", p)
}

fn session_path_now(paths: &Paths, mcu: &McuKind, ts: &Timestamp, auto: bool) -> PathBuf {
    let dir = paths.session_dir(mcu.clone());
    let suffix = if auto { "auto" } else { "sess" };
    let filename = format!("{}-{}.log", ts.wall.format("%Y%m%d_%H%M%S"), suffix);
    dir.join(filename)
}

fn latest_session(paths: &Paths, mcu: &McuKind) -> Result<Option<PathBuf>> {
    let dir = paths.session_dir(mcu.clone());
    if !dir.exists() {
        return Ok(None);
    }
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir)? {
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

fn query_session_logs(
    _paths: &Paths,
    meta_entries: &serde_json::Value,
    since: Option<String>,
    until: Option<String>,
    tail: Option<usize>,
) -> Result<serde_json::Value> {
    let mut results = Vec::new();
    if let Some(arr) = meta_entries.as_array() {
        for v in arr {
            if let Some(sess) = v.get("session").and_then(|s| s.as_str()) {
                let path = PathBuf::from(sess);
                if !path.exists() {
                    continue;
                }
                let lines = read_session(&path, since.as_deref(), until.as_deref(), tail)?;
                results.push(json!({"session": sess, "lines": lines}));
            }
        }
    }
    Ok(json!(results))
}

fn read_session(
    path: &PathBuf,
    since: Option<&str>,
    until: Option<&str>,
    tail: Option<usize>,
) -> Result<Vec<String>> {
    let reader = BufReader::new(File::open(path)?);
    let mut lines: Vec<String> = reader
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| session_ts_ok(l, since, until))
        .collect();
    if let Some(n) = tail {
        if lines.len() > n {
            lines = lines.split_off(lines.len() - n);
        }
    }
    Ok(lines)
}

fn session_ts_ok(line: &str, since: Option<&str>, until: Option<&str>) -> bool {
    // prefix looks like {"ts":"...","mcu":"..","event":".."} rest
    if let Some(end) = line.find('}') {
        let prefix = &line[..=end];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(prefix) {
            if let Some(ts) = v.get("ts").and_then(|t| t.as_str()) {
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
            }
        }
    }
    true
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

async fn tail_session(
    paths: &Paths,
    mcu: &McuKind,
    duration_ms: Option<u64>,
    max_lines: Option<usize>,
) -> Result<serde_json::Value> {
    let session = latest_session(paths, mcu)?
        .ok_or_else(|| anyhow::anyhow!("no session log for {:?}", mcu))?;

    let mut f = tokio::fs::OpenOptions::new()
        .read(true)
        .open(&session)
        .await?;
    // start tailing from end to only stream new lines
    f.seek(std::io::SeekFrom::End(0)).await?;
    let mut reader = TokioBuf::new(f);

    let start = tokio::time::Instant::now();
    let deadline = duration_ms.map(|d| start + tokio::time::Duration::from_millis(d));
    let mut lines = 0usize;
    let mut out = Vec::new();
    loop {
        let mut buf = String::new();
        let n = tokio::select! {
            res = reader.read_line(&mut buf) => res?,
            _ = async { if let Some(dl)=deadline { tokio::time::sleep_until(dl).await; } }, if deadline.is_some() => 0,
        };
        if n == 0 {
            if let Some(dl) = deadline {
                if tokio::time::Instant::now() >= dl {
                    break;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            continue;
        }
        out.push(buf.trim_end().to_string());
        lines += 1;
        if let Some(max) = max_lines {
            if lines >= max {
                break;
            }
        }
    }

    Ok(json!({
        "mcu": mcu,
        "session": session,
        "lines": out,
        "line_count": lines,
        "duration_ms": start.elapsed().as_millis(),
    }))
}
