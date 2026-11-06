//! Managed login enforcement detection (FR-06a): `forceLoginOrgUUID` and `forceLoginMethod`
//! from every managed-settings source Claude reads.

use crate::env::Env;
use qf_model::{identity_hash, ManagedLogin};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One raw settings document and where it came from.
pub struct Source {
    pub name: String,
    pub settings: Value,
    /// The source exists but could not be read or parsed. Its enforcement is unknown, so
    /// [`evaluate`] fails closed: no account is permitted until it is fixed.
    pub error: Option<String>,
}

/// `force_method` value recorded when a policy source exists but cannot be read; it
/// matches no login method, so every account is held back (fail closed).
pub const UNREADABLE_METHOD: &str = "unreadable-policy";

/// Parse a settings document. A leading UTF-8 byte order mark is ignored (Windows
/// editors add one); blank text is an empty document.
pub fn parse_settings(text: &str) -> Result<Value, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn source(name: String, parsed: Result<Value, String>) -> Source {
    match parsed {
        Ok(settings) => Source {
            name,
            settings,
            error: None,
        },
        Err(e) => Source {
            name,
            settings: Value::Null,
            error: Some(e),
        },
    }
}

fn managed_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    pf.join("ClaudeCode")
}

#[cfg(windows)]
fn registry_sources() -> Vec<Source> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut out = vec![];
    for (hive, label) in [
        (HKEY_LOCAL_MACHINE, "registry:HKLM"),
        (HKEY_CURRENT_USER, "registry:HKCU"),
    ] {
        if let Ok(key) = RegKey::predef(hive).open_subkey("SOFTWARE\\Policies\\ClaudeCode") {
            match key.get_value::<String, _>("Settings") {
                Ok(text) => out.push(source(label.into(), parse_settings(&text))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // Present but not a readable string: enforcement unknown.
                Err(e) => out.push(source(label.into(), Err(e.to_string()))),
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn registry_sources() -> Vec<Source> {
    vec![]
}

fn file_sources(dir: &Path) -> Vec<Source> {
    let mut out = vec![];
    let main = dir.join("managed-settings.json");
    if let Some(r) = read(&main) {
        out.push(source("file:managed-settings.json".into(), r));
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("managed-settings.d")) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for f in files {
            if let Some(r) = read(&f) {
                out.push(source(
                    format!(
                        "file:managed-settings.d\\{}",
                        f.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    r,
                ));
            }
        }
    }
    out
}

/// `None` when the file does not exist; otherwise the parsed document or why it is unusable.
fn read(p: &Path) -> Option<Result<Value, String>> {
    match std::fs::read(p) {
        Ok(bytes) => Some(
            String::from_utf8(bytes)
                .map_err(|e| e.to_string())
                .and_then(|t| parse_settings(&t)),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(Err(e.to_string())),
    }
}

/// All sources for an environment: registry, managed files, and the server-managed settings
/// Claude stored in the environment.
pub fn sources(env: &Env) -> Vec<Source> {
    let mut s = registry_sources();
    s.extend(file_sources(&managed_dir()));
    if let Some(r) = read(&env.config_dir.join("remote-settings.json")) {
        s.push(source("server-managed".into(), r));
    }
    s
}

/// Look for a key at the top level or one level down (for wrapped documents).
fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if let Some(x) = v.get(key) {
        return Some(x);
    }
    v.as_object()?
        .values()
        .filter(|x| x.is_object())
        .find_map(|x| x.get(key))
}

pub fn evaluate(sources: &[Source]) -> ManagedLogin {
    let mut m = ManagedLogin::default();
    let mut unreadable = false;
    for s in sources {
        if s.error.is_some() {
            // Exists but unusable: what it enforces is unknown (reported, never ignored).
            unreadable = true;
            m.sources.push(format!("{} (unreadable)", s.name));
            continue;
        }
        let mut hit = false;
        if let Some(v) = find(&s.settings, "forceLoginOrgUUID") {
            let orgs: Vec<String> = match v {
                Value::String(x) if !x.is_empty() => vec![x.clone()],
                Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(String::from).collect(),
                _ => vec![],
            };
            for o in orgs {
                let h = identity_hash(&o);
                if !m.force_org.contains(&h) {
                    m.force_org.push(h);
                }
                hit = true;
            }
        }
        if let Some(Value::String(method)) = find(&s.settings, "forceLoginMethod") {
            // The first (highest-precedence) source wins, as in Claude.
            if m.force_method.is_none() {
                m.force_method = Some(method.to_ascii_lowercase());
            }
            hit = true;
        }
        if hit {
            m.sources.push(s.name.clone());
        }
    }
    if unreadable {
        // Fail closed: a method no login matches, so no account is offered or switched into.
        m.force_method = Some(UNREADABLE_METHOD.into());
    }
    m
}

pub fn detect(env: &Env) -> ManagedLogin {
    evaluate(&sources(env))
}

/// Cheap fingerprint of the policy files to re-check after a change.
pub fn fingerprint(env: &Env) -> String {
    let mut parts = vec![];
    for p in [
        managed_dir().join("managed-settings.json"),
        env.config_dir.join("remote-settings.json"),
    ] {
        if let Ok(m) = std::fs::metadata(&p) {
            parts.push(format!("{:?}:{}", m.modified().ok(), m.len()));
        }
    }
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn evaluates_sources() {
        let m = evaluate(&[
            source("registry:HKLM".into(), Ok(json!({"forceLoginMethod": "claudeai"}))),
            source(
                "server-managed".into(),
                Ok(json!({"settings": {"forceLoginOrgUUID": ["o1", "o2"]}})),
            ),
            source("file".into(), Ok(json!({"forceLoginMethod": "console"}))),
            source("unrelated".into(), Ok(json!({"permissions": {}}))),
        ]);
        assert_eq!(m.force_method.as_deref(), Some("claudeai"));
        assert_eq!(m.force_org, vec![identity_hash("o1"), identity_hash("o2")]);
        assert_eq!(m.sources, vec!["registry:HKLM", "server-managed", "file"]);
        assert!(m.is_enforced());
        assert!(!evaluate(&[]).is_enforced());
    }

    fn account(org: &str) -> qf_model::Account {
        qf_model::Account {
            id: qf_model::AccountId("a".into()),
            label: "x".into(),
            kind: qf_model::AccountKind::Organization,
            org: identity_hash(org),
            subscription: None,
            rate_limit_tier: None,
            registered_at: 0,
            health: qf_model::Health::Ok,
            unavailable_models: vec![],
            org_name: None,
        }
    }

    #[test]
    fn bom_prefixed_managed_settings_are_enforced() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("managed-settings.json"),
            "\u{feff}{\"forceLoginOrgUUID\": \"o1\"}",
        )
        .unwrap();
        let m = evaluate(&file_sources(d.path()));
        assert_eq!(m.force_org, vec![identity_hash("o1")]);
        assert_eq!(m.sources, vec!["file:managed-settings.json"]);
        assert!(m.permits(&account("o1")));
        assert!(!m.permits(&account("o2")));
    }

    #[test]
    fn unreadable_managed_settings_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("managed-settings.json"), "{ \"forceLoginOrgUUID\": ").unwrap();
        let s = file_sources(d.path());
        assert_eq!(s.len(), 1);
        assert!(s[0].error.is_some());
        let m = evaluate(&s);
        assert!(m.is_enforced(), "an unparsable policy is never treated as no policy");
        assert_eq!(m.sources, vec!["file:managed-settings.json (unreadable)"]);
        assert!(!m.permits(&account("o1")));
        // Even when another source permits a method, the unknown one still holds everything back.
        let mut all = vec![source(
            "registry:HKLM".into(),
            Ok(json!({"forceLoginMethod": "claudeai"})),
        )];
        all.extend(s);
        assert!(!evaluate(&all).permits(&account("o1")));
        // A missing directory or blank file is no policy.
        assert!(file_sources(&d.path().join("absent")).is_empty());
        std::fs::write(d.path().join("managed-settings.json"), "\u{feff}  \n").unwrap();
        assert!(!evaluate(&file_sources(d.path())).is_enforced());
    }
}
