//! The data folder: `%LOCALAPPDATA%\Quayflow` on Windows, `$XDG_DATA_HOME/quayflow`
//! (`~/.local/share/quayflow`) on Linux. `QUAYFLOW_HOME` overrides it (tests).

use std::path::{Path, PathBuf};

/// The data folder the installer installs into on this platform.
pub fn default_root() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| qf_adapter::env::home_dir().join("AppData").join("Local"))
            .join("Quayflow")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| qf_adapter::env::home_dir().join(".local").join("share"))
            .join("quayflow")
    }
}

#[derive(Debug, Clone)]
pub struct DataDir {
    pub root: PathBuf,
}

impl DataDir {
    pub fn from_env() -> DataDir {
        if let Some(h) = std::env::var_os("QUAYFLOW_HOME").filter(|v| !v.is_empty()) {
            return DataDir { root: PathBuf::from(h) };
        }
        DataDir { root: default_root() }
    }

    pub fn at(root: &Path) -> DataDir {
        DataDir {
            root: root.to_path_buf(),
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [
            self.vault(),
            self.logs(),
            self.compat_dir(),
            self.lanes(),
            self.backups(),
            self.claims(),
            self.signin_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        // Logins, logs and the API token live here: owner-only on Linux.
        qf_vault::restrict_dir(&self.root)
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn vault(&self) -> PathBuf {
        self.root.join("vault")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn observations(&self) -> PathBuf {
        self.root.join("observations.json")
    }
    pub fn journal(&self) -> PathBuf {
        self.root.join("journal.jsonl")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn engine_json(&self) -> PathBuf {
        self.root.join("engine.json")
    }
    pub fn engine_lock(&self) -> PathBuf {
        self.root.join("engine.lock")
    }
    pub fn stopped_marker(&self) -> PathBuf {
        self.root.join("engine.stopped")
    }
    pub fn compat_dir(&self) -> PathBuf {
        self.root.join("compat")
    }
    pub fn lanes(&self) -> PathBuf {
        self.root.join("lanes")
    }
    pub fn signin_dir(&self) -> PathBuf {
        self.root.join("signin")
    }
    pub fn claims(&self) -> PathBuf {
        self.root.join("handoff").join("claims")
    }
    pub fn handoff_backups(&self) -> PathBuf {
        self.root.join("handoff").join("backups")
    }
    pub fn reports(&self) -> PathBuf {
        self.root.join("reports")
    }
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn shell(&self) -> PathBuf {
        self.root.join("shell")
    }
    pub fn simulate_limit(&self) -> PathBuf {
        self.root.join("simulate-limit.json")
    }
}

/// Connection info the Mod and CLI read.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConn {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub protocol: u32,
    pub version: String,
    pub started_at: i64,
}

impl EngineConn {
    pub fn read(dd: &DataDir) -> Option<EngineConn> {
        serde_json::from_str(&std::fs::read_to_string(dd.engine_json()).ok()?).ok()
    }
}
