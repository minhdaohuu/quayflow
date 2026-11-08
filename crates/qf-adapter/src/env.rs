//! Credential environment paths.

use qf_model::EnvId;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub id: EnvId,
    /// Claude's configuration folder (`~/.claude` or `CLAUDE_CONFIG_DIR`).
    pub config_dir: PathBuf,
    /// The global configuration file holding `oauthAccount`.
    pub claude_json: PathBuf,
}

pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Env {
    /// The shared default environment. Honors a user-wide `CLAUDE_CONFIG_DIR`,
    /// as Claude does: then `.claude.json` lives inside that folder.
    pub fn default_env() -> Env {
        match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => Env::at(EnvId::default_env(), PathBuf::from(dir)),
            None => Env::default_at(&home_dir()),
        }
    }

    /// Default layout under a given home folder (tests use a temporary home).
    pub fn default_at(home: &Path) -> Env {
        Env {
            id: EnvId::default_env(),
            config_dir: home.join(".claude"),
            claude_json: home.join(".claude.json"),
        }
    }

    /// A `CLAUDE_CONFIG_DIR`-style environment (lanes, sign-in environments).
    pub fn at(id: EnvId, dir: PathBuf) -> Env {
        Env {
            id,
            claude_json: dir.join(".claude.json"),
            config_dir: dir,
        }
    }

    pub fn credentials(&self) -> PathBuf {
        self.config_dir.join(".credentials.json")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.config_dir.join("sessions")
    }
    pub fn projects_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }
    /// `<configDir>\.oauth_refresh.lock`
    pub fn refresh_lock(&self) -> PathBuf {
        self.config_dir.join(".oauth_refresh.lock")
    }
    /// `<configDir>.lock` (sibling of the folder). Claude names it after the resolved folder
    /// (symbolic links followed), so on Unix the path is resolved when the folder exists.
    pub fn legacy_lock(&self) -> PathBuf {
        #[cfg(unix)]
        let dir = std::fs::canonicalize(&self.config_dir).unwrap_or_else(|_| self.config_dir.clone());
        #[cfg(not(unix))]
        let dir = self.config_dir.clone();
        let mut s = dir.into_os_string();
        s.push(".lock");
        PathBuf::from(s)
    }
    /// `<configDir>\.storage-write.lock`: Claude holds it around every write to its login store.
    pub fn storage_lock(&self) -> PathBuf {
        self.config_dir.join(".storage-write.lock")
    }
    /// `<.claude.json>.lock`
    pub fn config_lock(&self) -> PathBuf {
        let mut s = self.claude_json.as_os_str().to_owned();
        s.push(".lock");
        PathBuf::from(s)
    }
    /// The value of `CLAUDE_CONFIG_DIR` that selects this environment (None for the default).
    pub fn selector(&self) -> Option<&Path> {
        if self.id.is_default() {
            None
        } else {
            Some(&self.config_dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths() {
        let home = Path::new("home").join("u");
        let e = Env::default_at(&home);
        assert_eq!(e.credentials(), home.join(".claude").join(".credentials.json"));
        assert_eq!(e.legacy_lock(), home.join(".claude.lock"));
        assert_eq!(e.config_lock(), home.join(".claude.json.lock"));
        assert_eq!(e.refresh_lock(), home.join(".claude").join(".oauth_refresh.lock"));
        assert_eq!(e.storage_lock(), home.join(".claude").join(".storage-write.lock"));
        let lanes = Path::new("data").join("lanes");
        let l = Env::at(EnvId("work".into()), lanes.join("work"));
        assert_eq!(l.claude_json, lanes.join("work").join(".claude.json"));
        assert_eq!(l.legacy_lock(), lanes.join("work.lock"));
        assert!(l.selector().is_some());
    }
}
