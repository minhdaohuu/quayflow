//! Claude's session registry: `<configDir>\sessions\<pid>.json`.

use crate::env::Env;
use crate::process::{is_alive, start_time_ms};
use qf_model::SessionInfo;

/// Slack for clock and file-time granularity when comparing a process's start time with
/// the time its session file was written.
const START_SLACK_MS: i64 = 2_000;

/// Live sessions of an environment. Dead PIDs are skipped, and so are files whose PID now
/// belongs to a process started after the file was last written (a reused PID).
pub fn read_sessions(env: &Env) -> Vec<SessionInfo> {
    read_sessions_checked(env, |pid, written| {
        is_alive(pid) && !pid_reused(start_time_ms(pid), written)
    })
}

pub fn read_sessions_with(env: &Env, alive: impl Fn(u32) -> bool) -> Vec<SessionInfo> {
    read_sessions_checked(env, |pid, _| alive(pid))
}

/// A process that started after its session file was last written cannot have written it:
/// the PID was reused by another process.
fn pid_reused(started: Option<i64>, written: Option<i64>) -> bool {
    matches!((started, written), (Some(s), Some(w)) if s > w + START_SLACK_MS)
}

fn unix_ms(t: std::time::SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

/// `alive(pid, written)` gets the latest time (Unix ms) the session is known to have been
/// written: the file's modification time or its `startedAt`, whichever is later.
fn read_sessions_checked(env: &Env, alive: impl Fn(u32, Option<i64>) -> bool) -> Vec<SessionInfo> {
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
        let modified = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(unix_ms);
        let started_at = v.get("startedAt").and_then(|x| x.as_i64());
        let written = match (modified, started_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if !alive(pid, written) {
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

    #[test]
    fn reused_pid_is_not_a_live_session() {
        assert!(pid_reused(Some(10_000), Some(1_000)));
        assert!(!pid_reused(Some(1_000), Some(10_000)));
        assert!(!pid_reused(Some(1_500), Some(1_000)), "within the slack");
        assert!(!pid_reused(None, Some(1_000)));
        assert!(!pid_reused(Some(1_000), None));
    }

    #[cfg(windows)]
    #[test]
    fn stale_file_for_a_reused_pid_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let e = Env::default_at(d.path());
        std::fs::create_dir_all(e.sessions_dir()).unwrap();
        let pid = std::process::id();
        let f = e.sessions_dir().join(format!("{pid}.json"));
        std::fs::write(&f, format!(r#"{{"pid":{pid},"sessionId":"old"}}"#)).unwrap();
        assert_eq!(read_sessions(&e).len(), 1, "written after this process started");
        // Last written long before this process started: a previous owner of the PID.
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(read_sessions(&e).is_empty());
    }
}
