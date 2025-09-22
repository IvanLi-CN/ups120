use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Embed a monotonically increasing build timestamp into the binary
    // so we can verify that the flashed ELF is the one just built.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    println!("cargo:rustc-env=SB_BUILD_TS={}", ts);
}

