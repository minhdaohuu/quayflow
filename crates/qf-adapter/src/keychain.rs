//! Claude Code's login in the macOS Keychain (docs/tech-spec.md §1b).
//!
//! Claude keeps its login store (`claudeAiOauth` plus other keys such as `mcpOAuth`) as one
//! JSON object in a generic-password item. The service is `Claude Code-credentials`, or
//! `Claude Code-credentials-<first 8 hex of sha256(CLAUDE_CONFIG_DIR)>` when the variable is
//! set. Every access goes through `/usr/bin/security`, as Claude's own does: the item's access
//! list trusts that tool, and the Security framework would move the item out of the
//! `apple-tool:` partition, after which Claude meets a Keychain dialog. Secrets travel on
//! stdin, never on the command line.

use crate::env::Env;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const SECURITY: &str = "/usr/bin/security";
pub const BASE_SERVICE: &str = "Claude Code-credentials";
/// Claude's own limit for one `security -i` line; Claude falls back to argv above it, which
/// would put the login in the process list. Quayflow refuses instead.
pub const MAX_STDIN_LINE: usize = 4032;
/// Exit code of `security` for an item that does not exist (errSecItemNotFound).
const NOT_FOUND: i32 = 44;
/// How long one `security` call may take. A call that blocks is waiting on a dialog.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The Keychain service Claude uses for an environment, or `None` when Quayflow must not use
/// the Keychain for it.
///
/// The default environment (no `CLAUDE_CONFIG_DIR`: `.claude.json` beside `.claude`) uses the
/// bare name, which is one item per user, whatever `$HOME` says. So the bare name is only ever
/// used for the real user's own `~/.claude` (home from the account database, not `$HOME`);
/// a default-layout folder anywhere else (tests, fake environments) gets `None`. Otherwise the
/// suffix hashes the folder exactly as given, unresolved: a trailing slash or a path through a
/// symbolic link names another item, as in Claude.
pub fn service_name(env: &Env) -> Option<String> {
    if enabled_by_env() == Some(false) {
        return None;
    }
    service_name_for(env, account_home().as_deref())
}

/// [`service_name`] for a given real home folder (no environment variables read).
pub fn service_name_for(env: &Env, real_home: Option<&std::path::Path>) -> Option<String> {
    if env.claude_json != env.config_dir.join(".claude.json") {
        let real = real_home?;
        let is_real = env.config_dir == real.join(".claude") && env.claude_json == real.join(".claude.json");
        return is_real.then(|| BASE_SERVICE.to_string());
    }
    let raw = env.config_dir.as_os_str().to_string_lossy();
    let digest = Sha256::digest(raw.as_bytes());
    Some(format!("{BASE_SERVICE}-{}", &hex::encode(digest)[..8]))
}

/// The Keychain service a Claude session reads, from its variables exactly as set (what
/// Claude computes at start): `CLAUDE_SECURESTORAGE_CONFIG_DIR` first (empty: the bare name),
/// else `CLAUDE_CONFIG_DIR` (unset or empty: the bare name).
pub fn session_service(config_dir: Option<&str>, secure_storage_dir: Option<&str>) -> String {
    let source = match secure_storage_dir {
        Some(s) => s,
        None => config_dir.unwrap_or(""),
    };
    if source.is_empty() {
        return BASE_SERVICE.to_string();
    }
    format!(
        "{BASE_SERVICE}-{}",
        &hex::encode(Sha256::digest(source.as_bytes()))[..8]
    )
}

/// `QUAYFLOW_KEYCHAIN=off` keeps every environment file-backed (fake-environment tests).
fn enabled_by_env() -> Option<bool> {
    match std::env::var("QUAYFLOW_KEYCHAIN").ok()?.to_ascii_lowercase().as_str() {
        "off" | "0" | "false" | "no" => Some(false),
        _ => Some(true),
    }
}

/// The current user's home folder from the account database (never `$HOME`).
#[cfg(target_os = "macos")]
pub fn account_home() -> Option<std::path::PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: getpwuid_r fills `pwd` with pointers into `buf`, which outlives their use here.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16 * 1024];
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwuid_r(libc::getuid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out);
        if r != 0 || out.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        let dir = CStr::from_ptr(pwd.pw_dir).to_bytes();
        (!dir.is_empty()).then(|| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(dir)))
    }
}

/// Only macOS uses the Keychain.
#[cfg(not(target_os = "macos"))]
pub fn account_home() -> Option<std::path::PathBuf> {
    None
}

/// The account name Claude writes: `$USER` (else the login name), or `claude-code-user` when
/// it holds characters outside `[A-Za-z0-9._-]`.
pub fn default_account() -> String {
    let user = std::env::var("USER").ok().filter(|u| !u.is_empty());
    match user {
        Some(u) if valid_account(&u) => u,
        _ => "claude-code-user".to_string(),
    }
}

fn valid_account(a: &str) -> bool {
    !a.is_empty()
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Attributes of an existing item, read without its secret.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub service: String,
    pub account: String,
    /// Modification time as printed by `security` (`YYYYMMDDhhmmssZ`, one-second resolution).
    pub modified: String,
}

/// Run `security` with an optional stdin, giving up after `CALL_TIMEOUT`.
fn run(args: &[&str], stdin: Option<&str>) -> Result<(i32, String, String), String> {
    let mut child = Command::new(SECURITY)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{SECURITY}: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .map_err(|e| format!("{SECURITY}: {e}"))?;
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > CALL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the Keychain did not answer in time (a Keychain dialog may be waiting)".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{SECURITY}: {e}")),
        }
    }
    let out = child.wait_with_output().map_err(|e| format!("{SECURITY}: {e}"))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

fn attribute(out: &str, name: &str) -> Option<String> {
    let prefix = format!("\"{name}\"");
    out.lines().map(str::trim).find_map(|l| {
        let rest = l.strip_prefix(&prefix)?;
        let value = rest.split_once('=')?.1.trim();
        if let Some(q) = value.strip_prefix('"') {
            return q.strip_suffix('"').map(String::from);
        }
        // `<timedate>=0x… "20261108103049Z\000"`: the quoted part follows the hex form.
        let q = value.split_once('"')?.1;
        Some(q.split(['"', '\\']).next()?.to_string())
    })
}

/// The item for `service`, if any. No secret is read.
pub fn find(service: &str) -> Result<Option<Item>, String> {
    let (code, out, err) = run(&["find-generic-password", "-s", service], None)?;
    match code {
        0 => Ok(Some(Item {
            service: service.to_string(),
            account: attribute(&out, "acct").unwrap_or_default(),
            modified: attribute(&out, "mdat").unwrap_or_default(),
        })),
        NOT_FOUND => Ok(None),
        _ => Err(format!("the Keychain refused to look up the login ({code}: {err})")),
    }
}

/// `security -w` prints a password as text, or as hex when it holds a byte that is not
/// printable ASCII (any non-ASCII character in the JSON).
fn parse_password(out: &str) -> Option<Map<String, Value>> {
    let object = |s: &str| match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    };
    object(out).or_else(|| {
        let bytes = hex::decode(out).ok()?;
        object(std::str::from_utf8(&bytes).ok()?)
    })
}

/// The login store held by an item (`None` when the item does not exist).
pub fn read(item: &Item) -> Result<Option<Map<String, Value>>, String> {
    let (code, out, err) = run(
        &["find-generic-password", "-a", &item.account, "-s", &item.service, "-w"],
        None,
    )?;
    match code {
        0 => parse_password(out.trim())
            .map(Some)
            .ok_or_else(|| "the Keychain login is not a JSON object".into()),
        NOT_FOUND => Ok(None),
        _ => Err(format!("the Keychain refused to read the login ({code}: {err})")),
    }
}

/// Replace the item's login store (create the item if needed), as Claude writes it.
pub fn write(item: &Item, data: &Map<String, Value>) -> Result<(), String> {
    if !valid_account(&item.account) || item.service.contains('"') {
        return Err("unexpected Keychain item name".into());
    }
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    let line = format!(
        "add-generic-password -U -a \"{}\" -s \"{}\" -X \"{}\"\n",
        item.account,
        item.service,
        hex::encode(json.as_bytes())
    );
    if line.len() > MAX_STDIN_LINE {
        return Err(format!(
            "the login is too large to pass to the Keychain safely ({} bytes)",
            json.len()
        ));
    }
    let (code, _, err) = run(&["-i"], Some(&line))?;
    // `security -i` reports some failures only on stderr; the caller reads the item back.
    if code != 0 || err.contains("SecKeychain") || err.contains("error") {
        return Err(format!("the Keychain refused the login ({code}: {err})"));
    }
    Ok(())
}

/// Delete the item (its account and service only).
pub fn delete(item: &Item) -> Result<(), String> {
    let (code, _, err) = run(
        &["delete-generic-password", "-a", &item.account, "-s", &item.service],
        None,
    )?;
    match code {
        0 | NOT_FOUND => Ok(()),
        _ => Err(format!("the Keychain refused to delete the login ({code}: {err})")),
    }
}

/// The item Claude would create for this environment (`None` when it must not be used).
pub fn new_item(env: &Env) -> Option<Item> {
    Some(Item {
        service: service_name(env)?,
        account: default_account(),
        modified: String::new(),
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn passwords_printed_as_hex_are_read() {
        let m = parse_password("7b2261223a22636166c3a9227d").unwrap();
        assert_eq!(m["a"], "caf\u{e9}");
        assert_eq!(parse_password(r#"{"a":1}"#).unwrap()["a"], 1);
        assert!(parse_password("not json").is_none());
        assert!(parse_password("7b").is_none());
    }
    use super::*;
    use std::path::Path;

    #[test]
    fn service_names_match_claude() {
        let home = Path::new("/Users/u");
        // A default-layout folder that is not the real user's home never names the bare item
        // (one per user, whatever $HOME says): tests and fake environments stay file-backed.
        let d = tempfile::tempdir().unwrap();
        assert_eq!(service_name_for(&Env::default_at(d.path()), Some(home)), None);
        assert_eq!(
            service_name_for(&Env::default_at(Path::new("/Users/other")), Some(home)),
            None
        );
        assert_eq!(service_name_for(&Env::default_at(home), None), None);
        assert_eq!(
            service_name_for(&Env::default_at(home), Some(home)).as_deref(),
            Some(BASE_SERVICE)
        );
        assert_eq!(service_name(&Env::default_at(d.path())), None);
        // CLAUDE_CONFIG_DIR set, even to ~/.claude: hashed, unresolved, exactly as given.
        let e = Env::at(qf_model::EnvId("x".into()), "/Users/u/.claude".into());
        let expect = format!(
            "{BASE_SERVICE}-{}",
            &hex::encode(Sha256::digest(b"/Users/u/.claude"))[..8]
        );
        assert_eq!(service_name_for(&e, Some(home)), Some(expect.clone()));
        let slash = Env::at(qf_model::EnvId("x".into()), "/Users/u/.claude/".into());
        assert_ne!(
            service_name_for(&slash, Some(home)),
            Some(expect),
            "a trailing slash names another item"
        );
    }

    #[test]
    fn session_services_follow_claude() {
        assert_eq!(session_service(None, None), BASE_SERVICE);
        assert_eq!(session_service(Some(""), None), BASE_SERVICE);
        let e = Env::at(qf_model::EnvId("x".into()), "/Users/u/work".into());
        // A session started with the lane's exact folder reads the item Quayflow writes.
        assert_eq!(
            Some(session_service(Some("/Users/u/work"), None)),
            service_name_for(&e, Some(Path::new("/Users/u")))
        );
        assert_ne!(
            session_service(Some("/Users/u/work/"), None),
            session_service(Some("/Users/u/work"), None)
        );
        // The secure-storage folder wins; empty means the bare name.
        assert_eq!(session_service(Some("/Users/u/work"), Some("")), BASE_SERVICE);
        assert_eq!(session_service(None, Some("/s")), session_service(Some("/s"), None));
    }

    #[test]
    fn attributes_are_parsed() {
        let out = "keychain: \"/Users/u/Library/Keychains/login.keychain-db\"\nattributes:\n    \"acct\"<blob>=\"minhdao\"\n    \"mdat\"<timedate>=0x32303236313130383130333034395A00  \"20261108103049Z\\000\"\n    \"svce\"<blob>=\"Claude Code-credentials\"\n";
        assert_eq!(attribute(out, "acct").as_deref(), Some("minhdao"));
        assert_eq!(attribute(out, "mdat").as_deref(), Some("20261108103049Z"));
        assert_eq!(attribute(out, "svce").as_deref(), Some("Claude Code-credentials"));
        assert_eq!(attribute(out, "cdat"), None);
    }

    #[test]
    fn account_names() {
        assert!(valid_account("minh.dao_1-x"));
        assert!(!valid_account("a b"));
        assert!(!valid_account("a\"b"));
        assert!(!valid_account(""));
    }

    /// Round trip against the real Keychain with a throwaway item (macOS only; skipped when
    /// no Keychain is available, as on some CI runners).
    #[cfg(target_os = "macos")]
    #[test]
    fn keychain_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let env = Env::at(qf_model::EnvId("t".into()), d.path().join("env"));
        let Some(item) = new_item(&env) else {
            eprintln!("Keychain turned off; skipped");
            return;
        };
        assert_ne!(item.service, BASE_SERVICE);
        struct Cleanup(Item);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let _c = Cleanup(item.clone());
        if find(&item.service).is_err() {
            eprintln!("no usable Keychain; skipped");
            return;
        }
        assert_eq!(find(&item.service).unwrap(), None);
        let mut m = Map::new();
        m.insert(
            "claudeAiOauth".into(),
            serde_json::json!({"accessToken": "t", "refreshToken": "r"}),
        );
        m.insert("mcpOAuth".into(), serde_json::json!({"k": 1, "name": "caf\u{e9}"}));
        if write(&item, &m).is_err() {
            eprintln!("Keychain not writable; skipped");
            return;
        }
        let found = find(&item.service).unwrap().expect("item exists");
        assert_eq!(found.account, item.account);
        assert!(!found.modified.is_empty());
        assert_eq!(read(&found).unwrap(), Some(m));
        delete(&found).unwrap();
        assert_eq!(find(&item.service).unwrap(), None);
    }
}
