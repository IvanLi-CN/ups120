use crate::model::McuKind;
use crate::paths::Paths;
use anyhow::{Context, Result};
use std::fs;

pub fn read_port(paths: &Paths, mcu: McuKind) -> Result<Option<String>> {
    let p = match mcu {
        McuKind::Esp32 => &paths.esp32_port,
        McuKind::Stm32 => &paths.stm32_port,
    };
    if p.exists() {
        return Ok(Some(fs::read_to_string(p)?.trim().to_string()));
    }
    // legacy
    if mcu == McuKind::Stm32 && paths.stm32_legacy.exists() {
        return Ok(Some(
            fs::read_to_string(&paths.stm32_legacy)?.trim().to_string(),
        ));
    }
    Ok(None)
}

pub fn write_port(paths: &Paths, mcu: McuKind, val: &str) -> Result<()> {
    let p = match mcu {
        McuKind::Esp32 => &paths.esp32_port,
        McuKind::Stm32 => &paths.stm32_port,
    };
    fs::write(p, val.trim()).with_context(|| format!("write {:?}", p))?;
    Ok(())
}
