use std::{
    env,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn ci_short_hash() -> Option<String> {
    let sha = env::var("GITHUB_SHA").ok()?;
    if sha.is_empty() {
        return None;
    }
    Some(sha.chars().take(7).collect())
}

fn git_short_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn resolve_git_hash() -> String {
    git_short_hash()
        .or_else(ci_short_hash)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn build_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn main() {
    let git_hash = resolve_git_hash();
    println!("cargo:rustc-env=UPS_GIT_HASH={}", git_hash);

    let ts = build_timestamp();
    println!("cargo:rustc-env=UPS_BUILD_TS={}", ts);
}
