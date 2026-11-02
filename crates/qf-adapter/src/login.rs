//! Login read and atomic in-place swap (docs/tech-spec.md §3).
//!
//! Only `claudeAiOauth` in `.credentials.json` and `oauthAccount` in `.claude.json`
//! are replaced; every other key is kept in order.

use crate::env::Env;
use crate::lock::{self, LockError};
use qf_vault::{atomic_write, Login};
use serde_json::{Map, Value};
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum SwapError {
    /// Lock not obtained: nothing changed.
    Lock(LockError),
    /// The pre-write step (vault capture, journal) failed: nothing changed.
    PreWrite(String),
    /// Reading the current files failed: nothing changed.
    Read(String),
    /// A write failed or verification did not match. The state after this is uncertain.
    PostWrite(String),
}

impl SwapError {
    pub fn written(&self) -> bool {
        matches!(self, SwapError::PostWrite(_))
    }
}

impl fmt::Display for SwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwapError::Lock(e) => write!(f, "{e}"),
            SwapError::PreWrite(m) => write!(f, "not switched: {m}"),
            SwapError::Read(m) => write!(f, "not switched: cannot read the current login: {m}"),
            SwapError::PostWrite(m) => write!(f, "switch outcome uncertain: {m}"),
        }
    }
}

impl std::error::Error for SwapError {}

fn read_json(path: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(Map::new()),
        Ok(t) => match serde_json::from_str::<Value>(&t) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Err(format!("{} is not a JSON object", file_name(path))),
            Err(e) => Err(format!("{} is not valid JSON: {e}", file_name(path))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("{}: {e}", file_name(path))),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The environment's current login, or `None` when signed out.
pub fn read_login(env: &Env) -> Result<Option<Login>, String> {
    let creds = read_json(&env.credentials())?;
    let config = read_json(&env.claude_json)?;
    match (creds.get("claudeAiOauth"), config.get("oauthAccount")) {
        (Some(o), Some(a)) if o.is_object() && a.is_object() => Ok(Some(Login {
            oauth: o.clone(),
            account: a.clone(),
        })),
        _ => Ok(None),
    }
}

/// Account UUID from `.claude.json` only (no token is read).
pub fn read_identity(env: &Env) -> Option<String> {
    let config = read_json(&env.claude_json).ok()?;
    config
        .get("oauthAccount")?
        .get("accountUuid")?
        .as_str()
        .map(String::from)
}

/// Stamp of the credential file, to detect changes cheaply.
pub fn credentials_stamp(env: &Env) -> Option<(u128, u64)> {
    let m = std::fs::metadata(env.credentials()).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((t, m.len()))
}

#[derive(Debug)]
pub struct SwapOutcome {
    /// The login that was live before the write (possibly refreshed by Claude).
    pub previous: Option<Login>,
    pub locked_ms: u64,
    pub changed: bool,
    /// Stamp of the credential file right after our write (to recognize our own write).
    pub stamp: Option<(u128, u64)>,
}

/// Swap the environment's login to `target` under Claude's locks.
///
/// `before_write` runs under the locks with the current login; it must store the outgoing
/// login (vault capture) and record the transition. If it fails, nothing is written.
pub fn swap(
    env: &Env,
    target: &Login,
    wait_each: Duration,
    before_write: impl FnOnce(Option<&Login>) -> Result<(), String>,
) -> Result<SwapOutcome, SwapError> {
    let started = Instant::now();
    let held = lock::acquire_env(env, wait_each).map_err(SwapError::Lock)?;
    let t0 = Instant::now();
    let mut creds = read_json(&env.credentials()).map_err(SwapError::Read)?;
    let mut config = read_json(&env.claude_json).map_err(SwapError::Read)?;
    let previous = match (creds.get("claudeAiOauth"), config.get("oauthAccount")) {
        (Some(o), Some(a)) if o.is_object() && a.is_object() => Some(Login {
            oauth: o.clone(),
            account: a.clone(),
        }),
        _ => None,
    };
    if previous.as_ref().is_some_and(|p| p.same_tokens(target)) {
        drop(held);
        return Ok(SwapOutcome {
            previous,
            locked_ms: t0.elapsed().as_millis() as u64,
            changed: false,
            stamp: None,
        });
    }
    before_write(previous.as_ref()).map_err(SwapError::PreWrite)?;

    creds.insert("claudeAiOauth".into(), target.oauth.clone());
    config.insert("oauthAccount".into(), target.account.clone());
    let creds_text = serde_json::to_string(&Value::Object(creds)).map_err(|e| SwapError::PreWrite(e.to_string()))?;
    let config_text =
        serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| SwapError::PreWrite(e.to_string()))?;
    // Credentials first: a client that reads between the two writes sees the new token,
    // which is what decides the account of its request.
    atomic_write(&env.credentials(), creds_text.as_bytes())
        .map_err(|e| SwapError::PreWrite(format!("credential file not written: {e}")))?;
    if let Err(e) = atomic_write(&env.claude_json, config_text.as_bytes()) {
        return Err(SwapError::PostWrite(format!(
            "account record not written after the credential write: {e}"
        )));
    }
    let stamp = credentials_stamp(env);
    let after = read_login(env).map_err(SwapError::PostWrite)?;
    let verified = after
        .as_ref()
        .is_some_and(|a| a.account_uuid() == target.account_uuid() && a.refresh_token() == target.refresh_token());
    let locked_ms = t0.elapsed().as_millis() as u64;
    drop(held);
    if !verified {
        return Err(SwapError::PostWrite(
            "written login did not read back as the destination".into(),
        ));
    }
    let _ = started;
    Ok(SwapOutcome {
        previous,
        locked_ms,
        changed: true,
        stamp,
    })
}

/// Remove the login from an environment without signing out (retire a sign-in copy).
pub fn retire(env: &Env, wait_each: Duration) -> Result<(), SwapError> {
    let held = lock::acquire_env(env, wait_each).map_err(SwapError::Lock)?;
    let mut creds = read_json(&env.credentials()).map_err(SwapError::Read)?;
    let mut config = read_json(&env.claude_json).map_err(SwapError::Read)?;
    creds.remove("claudeAiOauth");
    config.remove("oauthAccount");
    let r = atomic_write(
        &env.credentials(),
        serde_json::to_string(&Value::Object(creds))
            .unwrap_or_default()
            .as_bytes(),
    )
    .and_then(|_| {
        atomic_write(
            &env.claude_json,
            serde_json::to_string_pretty(&Value::Object(config))
                .unwrap_or_default()
                .as_bytes(),
        )
    });
    drop(held);
    r.map_err(|e| SwapError::PostWrite(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, Env) {
        let d = tempfile::tempdir().unwrap();
        let e = Env::default_at(d.path());
        std::fs::create_dir_all(&e.config_dir).unwrap();
        std::fs::write(
            e.credentials(),
            json!({"claudeAiOauth": {"accessToken":"A1","refreshToken":"R1","expiresAt":1}, "mcpOAuth": {"x": 1}})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            &e.claude_json,
            serde_json::to_string_pretty(
                &json!({"numStartups": 5, "oauthAccount": {"accountUuid":"u-a"}, "projects": {"p": 1}}),
            )
            .unwrap(),
        )
        .unwrap();
        (d, e)
    }

    fn target() -> Login {
        Login {
            oauth: json!({"accessToken":"B1","refreshToken":"RB","expiresAt":2}),
            account: json!({"accountUuid":"u-b"}),
        }
    }

    #[test]
    fn swap_replaces_only_login_keys() {
        let (_d, e) = setup();
        let mut seen = None;
        let out = swap(&e, &target(), Duration::from_secs(1), |prev| {
            seen = prev.and_then(|p| p.account_uuid().map(String::from));
            Ok(())
        })
        .unwrap();
        assert!(out.changed);
        assert_eq!(seen.as_deref(), Some("u-a"));
        assert_eq!(out.previous.unwrap().refresh_token(), Some("R1"));
        let creds: Value = serde_json::from_str(&std::fs::read_to_string(e.credentials()).unwrap()).unwrap();
        assert_eq!(creds["mcpOAuth"]["x"], 1);
        assert_eq!(creds["claudeAiOauth"]["refreshToken"], "RB");
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&e.claude_json).unwrap()).unwrap();
        assert_eq!(cfg["numStartups"], 5);
        assert_eq!(cfg["projects"]["p"], 1);
        assert_eq!(cfg["oauthAccount"]["accountUuid"], "u-b");
        // Key order kept.
        let keys: Vec<&String> = cfg.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["numStartups", "oauthAccount", "projects"]);
        assert!(!e.refresh_lock().exists() && !e.config_lock().exists());
        assert_eq!(read_identity(&e).as_deref(), Some("u-b"));
    }

    #[test]
    fn pre_write_failure_changes_nothing() {
        let (_d, e) = setup();
        let before = std::fs::read(e.credentials()).unwrap();
        let r = swap(&e, &target(), Duration::from_secs(1), |_| {
            Err("journal unavailable".into())
        });
        assert!(matches!(r, Err(SwapError::PreWrite(_))));
        assert_eq!(std::fs::read(e.credentials()).unwrap(), before);
    }

    #[test]
    fn held_lock_changes_nothing() {
        let (_d, e) = setup();
        std::fs::create_dir(e.refresh_lock()).unwrap();
        let r = swap(&e, &target(), Duration::from_millis(400), |_| Ok(()));
        assert!(matches!(r, Err(SwapError::Lock(_))));
        assert_eq!(read_identity(&e).as_deref(), Some("u-a"));
    }

    #[test]
    fn same_login_is_a_no_op() {
        let (_d, e) = setup();
        let current = read_login(&e).unwrap().unwrap();
        let out = swap(&e, &current, Duration::from_secs(1), |_| panic!("must not write")).unwrap();
        assert!(!out.changed);
    }

    #[test]
    fn fresh_environment_and_retire() {
        let d = tempfile::tempdir().unwrap();
        let e = Env::at(qf_model::EnvId("lane".into()), d.path().join("lane"));
        assert!(read_login(&e).unwrap().is_none());
        swap(&e, &target(), Duration::from_secs(1), |prev| {
            assert!(prev.is_none());
            Ok(())
        })
        .unwrap();
        assert_eq!(read_login(&e).unwrap().unwrap().account_uuid(), Some("u-b"));
        retire(&e, Duration::from_secs(1)).unwrap();
        assert!(read_login(&e).unwrap().is_none());
    }
}
