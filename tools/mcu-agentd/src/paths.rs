use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub sock: PathBuf,
    pub lock: PathBuf,
    pub logs_dir: PathBuf,
    pub meta_esp32: PathBuf,
    pub meta_stm32: PathBuf,
    pub session_esp32: PathBuf,
    pub session_stm32: PathBuf,
    pub esp32_port: PathBuf,
    pub stm32_port: PathBuf,
    pub stm32_legacy: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let logs_dir = root.join("logs/agentd");
        let sock = logs_dir.join("agentd.sock");
        let lock = logs_dir.join("agentd.lock");
        let meta_esp32 = logs_dir.join("esp32.meta.log");
        let meta_stm32 = logs_dir.join("stm32.meta.log");
        let session_esp32 = logs_dir.join("esp32");
        let session_stm32 = logs_dir.join("stm32");
        let esp32_port = root.join(".esp32-port");
        let stm32_port = root.join(".stm32-port");
        let stm32_legacy = root.join(".stm32-probe");
        Ok(Self {
            root,
            sock,
            lock,
            logs_dir,
            meta_esp32,
            meta_stm32,
            session_esp32,
            session_stm32,
            esp32_port,
            stm32_port,
            stm32_legacy,
        })
    }
    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.logs_dir)?;
        std::fs::create_dir_all(&self.session_esp32)?;
        std::fs::create_dir_all(&self.session_stm32)?;
        Ok(())
    }
    pub fn root(&self) -> &PathBuf {
        &self.root
    }
    pub fn meta(&self, mcu: crate::model::McuKind) -> &Path {
        match mcu {
            crate::model::McuKind::Esp32 => self.meta_esp32.as_path(),
            crate::model::McuKind::Stm32 => self.meta_stm32.as_path(),
        }
    }
    pub fn session_dir(&self, mcu: crate::model::McuKind) -> &Path {
        match mcu {
            crate::model::McuKind::Esp32 => self.session_esp32.as_path(),
            crate::model::McuKind::Stm32 => self.session_stm32.as_path(),
        }
    }
    pub fn lock_path(&self) -> &Path {
        self.lock.as_path()
    }

    pub fn auto_pid(&self, mcu: crate::model::McuKind) -> PathBuf {
        let name = match mcu {
            crate::model::McuKind::Esp32 => "auto-esp32.pid",
            crate::model::McuKind::Stm32 => "auto-stm32.pid",
        };
        self.logs_dir.join(name)
    }
}
