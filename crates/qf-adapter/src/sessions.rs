//! Claude's session registry: `<configDir>\sessions\<pid>.json`.

use crate::env::Env;
use crate::process::is_alive;
use qf_model::SessionInfo;

/// Live sessions of an environment (dead PIDs are skipped).
pub fn read_sessions(env: &Env) -> Vec<SessionInfo> {
    read_sessions_with(env, is_alive)
}

pub fn read_sessions_with(env: &Env, alive: impl Fn(u32) -> bool) -> Vec<SessionInfo> {
    let Ok(dir) = std::fs::read_dir(env.sessions_dir()) else {
        return vec![];
    };
    let mut out = vec![];
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if !stem.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(pid) = v.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32) else {
            continue;
        };
        if !alive(pid) {
            continue;
        }
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        out.push(SessionInfo {
            pid,
            session_id: s("sessionId").unwrap_or_default(),
            env: env.id.clone(),
            name: s("name"),
            kind: s("kind"),
            status: s("status"),
            cwd: s("cwd"),
            version: s("version"),
            group: None,
        });
    }
    out.sort_by_key(|s| s.pid);
    out
}

/// Oldest running Claude version among sessions.
pub fn oldest_version(sessions: &[SessionInfo]) -> Option<String> {
    sessions
        .iter()
        .filter_map(|s| s.version.clone())
        .min_by(|a, b| qf_model::compare_versions(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_fixture_files() {
        let d = tempfile::tempdir().unwrap();
        let e = Env::default_at(d.path());
        std::fs::create_dir_all(e.sessions_dir()).unwrap();
        std::fs::write(
            e.sessions_dir().join("100.json"),
            r#"{"pid":100,"sessionId":"s1","cwd":"C:\\p","version":"2.1.316","kind":"interactive","status":"busy","name":"one"}"#,
        )
        .unwrap();
        std::fs::write(
            e.sessions_dir().join("200.json"),
            r#"{"pid":200,"sessionId":"s2","version":"2.1.288","status":"idle"}"#,
        )
        .unwrap();
        std::fs::write(e.sessions_dir().join("300.json"), r#"{"pid":300,"sessionId":"dead"}"#).unwrap();
        std::fs::write(e.sessions_dir().join("junk.json"), "{}").unwrap();
        let s = read_sessions_with(&e, |p| p != 300);
        assert_eq!(s.len(), 2);
        assert!(s[0].is_busy());
        assert_eq!(oldest_version(&s).as_deref(), Some("2.1.288"));
    }
}
