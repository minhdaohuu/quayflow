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
            if let Ok(text) = key.get_value::<String, _>("Settings") {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    out.push(Source {
                        name: label.into(),
                        settings: v,
                    });
                }
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
    if let Some(v) = read(&main) {
        out.push(Source {
            name: "file:managed-settings.json".into(),
            settings: v,
        });
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("managed-settings.d")) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for f in files {
            if let Some(v) = read(&f) {
                out.push(Source {
                    name: format!(
                        "file:managed-settings.d\\{}",
                        f.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    settings: v,
                });
            }
        }
    }
    out
}

fn read(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// All sources for an environment: registry, managed files, and the server-managed settings
/// Claude stored in the environment.
pub fn sources(env: &Env) -> Vec<Source> {
    let mut s = registry_sources();
    s.extend(file_sources(&managed_dir()));
    if let Some(v) = read(&env.config_dir.join("remote-settings.json")) {
        s.push(Source {
            name: "server-managed".into(),
            settings: v,
        });
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
    for s in sources {
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
            Source {
                name: "registry:HKLM".into(),
                settings: json!({"forceLoginMethod": "claudeai"}),
            },
            Source {
                name: "server-managed".into(),
                settings: json!({"settings": {"forceLoginOrgUUID": ["o1", "o2"]}}),
            },
            Source {
                name: "file".into(),
                settings: json!({"forceLoginMethod": "console"}),
            },
            Source {
                name: "unrelated".into(),
                settings: json!({"permissions": {}}),
            },
        ]);
        assert_eq!(m.force_method.as_deref(), Some("claudeai"));
        assert_eq!(m.force_org, vec![identity_hash("o1"), identity_hash("o2")]);
        assert_eq!(m.sources, vec!["registry:HKLM", "server-managed", "file"]);
        assert!(m.is_enforced());
        assert!(!evaluate(&[]).is_enforced());
    }
}
