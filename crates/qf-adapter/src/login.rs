//! Login read and atomic in-place swap (docs/tech-spec.md §3, §1b).
//!
//! Only `claudeAiOauth` in the login store and `oauthAccount` in `.claude.json` are
//! replaced; every other key is kept in order. The login store is `.credentials.json`, or on
//! macOS the environment's Keychain item when it has one (Claude reads the Keychain first and
//! the file only when the Keychain holds nothing).

use crate::env::Env;
use crate::keychain;
use crate::lock::{self, LockError};
use qf_vault::{atomic_write, Login};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Where an environment's login lives.
#[derive(Debug, Clone, PartialEq)]
pub enum Store {
    /// `<configDir>/.credentials.json` (Windows, Linux, and Claude's fallback on macOS).
    File,
    /// The environment's Keychain item (macOS).
    Keychain(keychain::Item),
}

/// The login store Claude reads for this environment. On macOS that is the Keychain item when
/// one exists; an environment without one is read from `.credentials.json`, as Claude does,
/// and stays file-backed until Claude itself moves it into the Keychain at its next save.
pub fn store(env: &Env) -> Result<Store, String> {
    if !cfg!(target_os = "macos") {
        return Ok(Store::File);
    }
    let Some(service) = keychain::service_name(env) else {
        return Ok(Store::File);
    };
    Ok(match keychain::find(&service)? {
        Some(item) => Store::Keychain(item),
        None => Store::File,
    })
}

fn read_store(env: &Env, store: &Store) -> Result<Map<String, Value>, String> {
    match store {
        Store::File => read_json(&env.credentials()),
        Store::Keychain(item) => Ok(keychain::read(item)?.unwrap_or_default()),
    }
}

/// How long the marker stays after a Keychain switch: Claude caches a Keychain read for 30 s,
/// so after this every running session has read the new login either way.
pub const MARKER_LIFETIME: Duration = Duration::from_secs(35);

/// Marker bytes written per environment by this process, to recognize our own marker.
static MARKERS: Mutex<Option<HashMap<PathBuf, Vec<u8>>>> = Mutex::new(None);

/// After a Keychain write: put the same login store into `.credentials.json` (owner-only).
/// Claude checks that file's modification time before each request; a change makes it drop
/// its 30 s Keychain cache, so every running session uses the new login from its next request.
/// The file holds the real login, not an empty object, so a Keychain read that fails while
/// it exists still finds the right login.
fn write_marker(env: &Env, data: &Map<String, Value>) -> Result<(), String> {
    let bytes = serde_json::to_vec(data).map_err(|e| e.to_string())?;
    atomic_write(&env.credentials(), &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(env.credentials(), std::fs::Permissions::from_mode(0o600));
    }
    if let Ok(mut g) = MARKERS.lock() {
        g.get_or_insert_with(HashMap::new).insert(env.credentials(), bytes);
    }
    Ok(())
}

/// Remove the marker of a Keychain environment once it is `max_age` old, and only when it is
/// still ours: the bytes this process wrote, or the same login store as the Keychain item.
/// Anything else (Claude's own fallback login) is left alone. Returns whether it was removed.
pub fn sweep_marker(env: &Env, max_age: Duration) -> bool {
    let path = env.credentials();
    let Ok(meta) = std::fs::metadata(&path) else {
        return false;
    };
    let old = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|a| a >= max_age);
    if !old {
        return false;
    }
    let Ok(Store::Keychain(item)) = store(env) else {
        return false;
    };
    // Claude writes its fallback login under the storage lock; never race it.
    let Ok(held) = lock::acquire(&lock::specs(env)[2..3], Duration::from_millis(500)) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(&path) else { return false };
    let ours = MARKERS
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&path).cloned()))
        .is_some_and(|b| b == bytes);
    let same_as_keychain = || {
        let file: Option<Map<String, Value>> = serde_json::from_slice(&bytes).ok();
        file.is_some() && keychain::read(&item).ok().flatten() == file
    };
    let removed = (ours || same_as_keychain()) && std::fs::remove_file(&path).is_ok();
    drop(held);
    if removed {
        if let Ok(mut g) = MARKERS.lock() {
            if let Some(m) = g.as_mut() {
                m.remove(&path);
            }
        }
    }
    removed
}

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
        Ok(t) if t.trim_start_matches('\u{feff}').trim().is_empty() => Ok(Map::new()),
        // A leading UTF-8 byte order mark (added by some Windows editors) is not JSON.
        Ok(t) => match serde_json::from_str::<Value>(t.strip_prefix('\u{feff}').unwrap_or(&t)) {
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
    let creds = read_store(env, &store(env)?)?;
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

/// Stamp of the login store, to detect changes cheaply (no secret is read). For a Keychain
/// item it is the item's modification time (one-second resolution).
pub fn credentials_stamp(env: &Env) -> Option<(u128, u64)> {
    if let Ok(Store::Keychain(item)) = store(env) {
        let digits: String = item.modified.chars().filter(char::is_ascii_digit).collect();
        return digits.parse::<u128>().ok().map(|t| (t, 1));
    }
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
    swap_with(env, target, wait_each, false, before_write)
}

/// [`swap`], with `repair`: write the destination even when the live account record already
/// names it. The caller uses this when it knows the files are inconsistent (credentials of
/// another account under this account record), where skipping the write would keep them so.
pub fn swap_with(
    env: &Env,
    target: &Login,
    wait_each: Duration,
    repair: bool,
    before_write: impl FnOnce(Option<&Login>) -> Result<(), String>,
) -> Result<SwapOutcome, SwapError> {
    let started = Instant::now();
    let held = lock::acquire_env(env, wait_each).map_err(SwapError::Lock)?;
    let t0 = Instant::now();
    let store = store(env).map_err(SwapError::Read)?;
    let mut creds = read_store(env, &store).map_err(SwapError::Read)?;
    let mut config = read_json(&env.claude_json).map_err(SwapError::Read)?;
    let previous = match (creds.get("claudeAiOauth"), config.get("oauthAccount")) {
        (Some(o), Some(a)) if o.is_object() && a.is_object() => Some(Login {
            oauth: o.clone(),
            account: a.clone(),
        }),
        _ => None,
    };
    // Already on the destination account (identity, not tokens): Claude may have refreshed
    // the live tokens since the vault copy was taken, so writing the copy would replace fresh
    // tokens with older ones. Leave the files alone; `previous` carries the live (fresh)
    // login for the caller to capture.
    let same_identity = previous.as_ref().is_some_and(|p| {
        p.same_tokens(target) || (!repair && p.account_uuid().is_some() && p.account_uuid() == target.account_uuid())
    });
    if same_identity {
        drop(held);
        return Ok(SwapOutcome {
            previous,
            locked_ms: t0.elapsed().as_millis() as u64,
            changed: false,
            stamp: None,
        });
    }
    before_write(previous.as_ref()).map_err(SwapError::PreWrite)?;
    if let Store::Keychain(item) = &store {
        return swap_keychain(env, item, creds, config, target, previous, held, t0);
    }

    creds.insert("claudeAiOauth".into(), target.oauth.clone());
    config.insert("oauthAccount".into(), target.account.clone());
    let creds_text = serde_json::to_string(&Value::Object(creds)).map_err(|e| SwapError::PreWrite(e.to_string()))?;
    let config_text =
        serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| SwapError::PreWrite(e.to_string()))?;
    // The credential file as it was, to put back if the account record cannot be written.
    let creds_before = match std::fs::read(env.credentials()) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(SwapError::Read(format!("{}: {e}", file_name(&env.credentials())))),
    };
    // Credentials first: a client that reads between the two writes sees the new token,
    // which is what decides the account of its request.
    atomic_write(&env.credentials(), creds_text.as_bytes())
        .map_err(|e| SwapError::PreWrite(format!("credential file not written: {e}")))?;
    if let Err(e) = atomic_write(&env.claude_json, config_text.as_bytes()) {
        // Roll the credential write back so tokens and account record never disagree.
        let restored = match &creds_before {
            Some(b) => atomic_write(&env.credentials(), b),
            None => std::fs::remove_file(env.credentials()),
        };
        return Err(match restored {
            Ok(()) => SwapError::PreWrite(format!(
                "account record not written ({e}); the credential write was rolled back"
            )),
            Err(r) => SwapError::PostWrite(format!(
                "account record not written after the credential write: {e}; rollback failed: {r}"
            )),
        });
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

/// The Keychain half of [`swap_with`], under the same locks: write the item, read it back,
/// write `oauthAccount`, then the marker that makes running sessions re-read at once.
#[allow(clippy::too_many_arguments)]
fn swap_keychain(
    env: &Env,
    item: &keychain::Item,
    mut creds: Map<String, Value>,
    mut config: Map<String, Value>,
    target: &Login,
    previous: Option<Login>,
    held: lock::HeldLocks,
    t0: Instant,
) -> Result<SwapOutcome, SwapError> {
    let before = creds.clone();
    creds.insert("claudeAiOauth".into(), target.oauth.clone());
    config.insert("oauthAccount".into(), target.account.clone());
    let config_text =
        serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| SwapError::PreWrite(e.to_string()))?;
    if let Err(e) = keychain::write(item, &creds) {
        // `-U` replaces the item in one step; make sure nothing changed before saying so.
        return Err(match keychain::read(item) {
            Ok(Some(now)) if now == before => SwapError::PreWrite(format!("Keychain login not written: {e}")),
            _ => SwapError::PostWrite(format!("Keychain write failed and its state is unclear: {e}")),
        });
    }
    let written = keychain::read(item);
    if written.as_ref().ok().and_then(|w| w.as_ref()) != Some(&creds) {
        // Put the Keychain back so tokens and account record never disagree.
        let why = match written {
            Err(e) => format!("the Keychain login could not be read back ({e})"),
            Ok(_) => "the Keychain login did not read back as written".into(),
        };
        return Err(match keychain::write(item, &before) {
            Ok(()) if keychain::read(item).ok().flatten().as_ref() == Some(&before) => {
                SwapError::PreWrite(format!("{why}; the Keychain write was rolled back"))
            }
            _ => SwapError::PostWrite(format!("{why}; rollback failed")),
        });
    }
    if let Err(e) = atomic_write(&env.claude_json, config_text.as_bytes()) {
        // Put the Keychain back so tokens and account record never disagree.
        return Err(match keychain::write(item, &before) {
            Ok(()) => SwapError::PreWrite(format!(
                "account record not written ({e}); the Keychain write was rolled back"
            )),
            Err(r) => SwapError::PostWrite(format!(
                "account record not written after the Keychain write: {e}; rollback failed: {r}"
            )),
        });
    }
    // Without the marker the switch still lands, within Claude's 30 s Keychain cache.
    let _ = write_marker(env, &creds);
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
    // A Keychain that cannot be asked (locked, a dialog, a timeout) is an error, never "no
    // item": the login would stay behind under a name nothing can find once the folder goes.
    if let Store::Keychain(item) = store(env).map_err(SwapError::Read)? {
        // The item first: keep its other keys, delete it when nothing else is in it.
        let mut kc = keychain::read(&item).map_err(SwapError::Read)?.unwrap_or_default();
        kc.remove("claudeAiOauth");
        let r = if kc.is_empty() {
            keychain::delete(&item)
        } else {
            keychain::write(&item, &kc)
        };
        r.map_err(SwapError::PostWrite)?;
    }
    let mut creds = read_json(&env.credentials()).map_err(SwapError::Read)?;
    let mut config = read_json(&env.claude_json).map_err(SwapError::Read)?;
    creds.remove("claudeAiOauth");
    config.remove("oauthAccount");
    // Rewrite the credential file only where one exists: an empty one would be a login store
    // that holds nothing, which Claude on macOS could fall back to.
    let r = if env.credentials().exists() {
        atomic_write(
            &env.credentials(),
            serde_json::to_string(&Value::Object(creds))
                .unwrap_or_default()
                .as_bytes(),
        )
    } else {
        Ok(())
    }
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
    fn bom_prefixed_files_are_read() {
        let (_d, e) = setup();
        for p in [e.credentials(), e.claude_json.clone()] {
            let t = std::fs::read_to_string(&p).unwrap();
            std::fs::write(&p, format!("\u{feff}{t}")).unwrap();
        }
        let l = read_login(&e).unwrap().unwrap();
        assert_eq!(l.account_uuid(), Some("u-a"));
        assert_eq!(read_identity(&e).as_deref(), Some("u-a"));
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
    fn same_account_with_refreshed_tokens_is_not_overwritten() {
        let (_d, e) = setup();
        // The vault copy of u-a is older than what Claude refreshed into the live files.
        let stale = Login {
            oauth: json!({"accessToken":"A0","refreshToken":"R0","expiresAt":0}),
            account: json!({"accountUuid":"u-a"}),
        };
        let before = std::fs::read(e.credentials()).unwrap();
        let out = swap(&e, &stale, Duration::from_secs(1), |_| panic!("must not write")).unwrap();
        assert!(!out.changed);
        assert_eq!(out.previous.unwrap().refresh_token(), Some("R1"));
        assert_eq!(std::fs::read(e.credentials()).unwrap(), before);
    }

    #[cfg(windows)]
    #[test]
    fn account_record_failure_rolls_back_credentials() {
        let (_d, e) = setup();
        let before = std::fs::read(e.credentials()).unwrap();
        // A read-only .claude.json cannot be replaced.
        let mut perm = std::fs::metadata(&e.claude_json).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&e.claude_json, perm.clone()).unwrap();
        let r = swap(&e, &target(), Duration::from_secs(1), |_| Ok(()));
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&e.claude_json, perm).unwrap();
        let err = r.unwrap_err();
        assert!(matches!(err, SwapError::PreWrite(_)), "{err}");
        assert!(!err.written());
        assert_eq!(std::fs::read(e.credentials()).unwrap(), before);
        assert_eq!(read_login(&e).unwrap().unwrap().refresh_token(), Some("R1"));
        assert_eq!(read_identity(&e).as_deref(), Some("u-a"));
    }

    /// macOS: an environment whose login Claude moved into the Keychain (a lane after its first
    /// refresh) is read and retired there; other entries of the item are kept, and an item left
    /// empty is deleted, so deleting a lane leaves no login behind. Throwaway item only.
    #[cfg(target_os = "macos")]
    #[test]
    fn keychain_environment_is_read_and_retired() {
        let d = tempfile::tempdir().unwrap();
        let e = Env::at(qf_model::EnvId("lane".into()), d.path().join("lane"));
        std::fs::create_dir_all(&e.config_dir).unwrap();
        let Some(item) = keychain::new_item(&e) else {
            eprintln!("Keychain turned off; skipped");
            return;
        };
        assert_ne!(item.service, keychain::BASE_SERVICE);
        struct Cleanup(keychain::Item);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = keychain::delete(&self.0);
            }
        }
        let _c = Cleanup(item.clone());
        let mut m = Map::new();
        m.insert(
            "claudeAiOauth".into(),
            json!({"accessToken":"A1","refreshToken":"R1","expiresAt":1}),
        );
        if keychain::write(&item, &m).is_err() {
            eprintln!("Keychain not writable; skipped");
            return;
        }
        std::fs::write(
            &e.claude_json,
            json!({"oauthAccount": {"accountUuid": "u-a"}}).to_string(),
        )
        .unwrap();
        assert!(matches!(store(&e).unwrap(), Store::Keychain(_)));
        assert_eq!(read_login(&e).unwrap().unwrap().refresh_token(), Some("R1"));
        // A switch writes the item and leaves the marker; the item's other entries stay.
        m.insert("mcpOAuth".into(), json!({"k": 1}));
        keychain::write(&item, &m).unwrap();
        let out = swap(&e, &target(), Duration::from_secs(2), |_| Ok(())).unwrap();
        assert!(out.changed);
        let found = keychain::find(&item.service).unwrap().unwrap();
        let now = keychain::read(&found).unwrap().unwrap();
        assert_eq!(now["claudeAiOauth"]["refreshToken"], "RB");
        assert_eq!(now["mcpOAuth"]["k"], 1);
        assert!(e.credentials().exists(), "marker written");
        // The marker goes only when it is ours (or equals the item) and old enough.
        assert!(!sweep_marker(&e, MARKER_LIFETIME));
        assert!(sweep_marker(&e, Duration::ZERO));
        assert!(!e.credentials().exists());
        // Retiring keeps the other entries, then deletes the item once nothing is left in it.
        retire(&e, Duration::from_secs(2)).unwrap();
        let kept = keychain::read(&keychain::find(&item.service).unwrap().unwrap())
            .unwrap()
            .unwrap();
        assert!(kept.get("claudeAiOauth").is_none() && kept.contains_key("mcpOAuth"));
        let mut only = Map::new();
        only.insert("claudeAiOauth".into(), json!({"refreshToken": "R2"}));
        keychain::write(&item, &only).unwrap();
        retire(&e, Duration::from_secs(2)).unwrap();
        assert_eq!(keychain::find(&item.service).unwrap(), None, "an empty item is deleted");
        assert!(!e.credentials().exists(), "no empty login file is created");
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
