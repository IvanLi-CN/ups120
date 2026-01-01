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
}

impl Paths {
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let mut root = cwd.clone();
        // Detect repo root by presence of Justfile + firmware dir (workspace root),
        // fallback to parent of tools/mcu-agentd when not found.
        for dir in cwd.ancestors() {
            let jf = dir.join("Justfile");
            let fw = dir.join("firmware");
            if jf.exists() && fw.exists() {
                root = dir.to_path_buf();
                break;
            }
        }
        if root == cwd {
            if let Some(parent) = cwd.parent() {
                root = parent.to_path_buf();
            }
        }
        let logs_dir = root.join("logs/agentd");
        let sock = logs_dir.join("agentd.sock");
        let lock = logs_dir.join("agentd.lock");
        let meta_esp32 = logs_dir.join("esp32.meta.log");
        let meta_stm32 = logs_dir.join("stm32.meta.log");
        let session_esp32 = logs_dir.join("esp32");
        let session_stm32 = logs_dir.join("stm32");
        let esp32_port = root.join(".esp32-port");
        let stm32_port = root.join(".stm32-port");
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

    pub fn monitor_pid(&self, mcu: crate::model::McuKind) -> PathBuf {
        let name = match mcu {
            crate::model::McuKind::Esp32 => "mon-esp32.pid",
            crate::model::McuKind::Stm32 => "mon-stm32.pid",
        };
        self.logs_dir.join(name)
    }
}
