use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    println!("cargo:rustc-env=SB_DEMO_BUILD_TS={}", ts);
}

