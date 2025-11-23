use crate::model::McuKind;
use crate::paths::Paths;
use crate::timefmt::Timestamp;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::Instant;

#[derive(Debug)]
pub struct RunResult {
    pub status: i32,
    pub duration_ms: u128,
    pub session_file: PathBuf,
}

pub async fn run_mcu_cmd(
    paths: &Paths,
    mcu: &McuKind,
    ts: &Timestamp,
    mut cmd: Command,
) -> Result<RunResult> {
    let session_file = session_file_path(paths, mcu, ts);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_file)
        .with_context(|| format!("open session log {:?}", session_file))?;

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().context("spawn command")?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let mut out_reader = BufReader::new(stdout).lines();
    let mut err_reader = BufReader::new(stderr).lines();

    let start = Instant::now();
    loop {
        tokio::select! {
            line = out_reader.next_line() => {
                match line? {
                    Some(l) => {
                        let pref = prefix(&ts.iso(), mcu, "log");
                        writeln!(file, "{} {}", pref, l)?;
                    }
                    None => break,
                }
            }
            line = err_reader.next_line() => {
                match line? {
                    Some(l) => {
                        let pref = prefix(&ts.iso(), mcu, "err");
                        writeln!(file, "{} {}", pref, l)?;
                    }
                    None => break,
                }
            }
        }
    }

    let status = child.wait().await?;
    let dur = start.elapsed();

    Ok(RunResult {
        status: status.code().unwrap_or(-1),
        duration_ms: dur.as_millis(),
        session_file,
    })
}

fn session_file_path(paths: &Paths, mcu: &McuKind, ts: &Timestamp) -> PathBuf {
    let dir = paths.session_dir(mcu.clone());
    let filename = format!("{}.session.log", ts.wall.format("%Y%m%d_%H%M%S"));
    dir.join(filename)
}

fn prefix(ts: &str, mcu: &McuKind, event: &str) -> String {
    format!(
        "{{\"ts\":\"{}\",\"mcu\":\"{}\",\"event\":\"{}\"}}",
        ts,
        match mcu {
            McuKind::Esp32 => "esp32",
            McuKind::Stm32 => "stm32",
        },
        event
    )
}
