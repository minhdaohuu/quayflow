//! Durable journal and sanitized logs (FR-22).
//!
//! The journal is append-only JSONL, fsynced before a transition writes. Logs rotate per
//! day, keep 7 days, and stop at a size cap. Both hold opaque ids and sanitized reasons
//! only: never tokens, prompts, responses, emails, labels, raw arguments or provider bodies.

use qf_model::time::{day_stamp, now_ms};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Remove anything that looks like a credential, and replace known labels by ids.
pub fn scrub(text: &str, labels: &[(String, String)]) -> String {
    scrub_secrets(&replace_labels(text, labels))
}

/// Replace whole-word occurrences of account labels by `#id`. A label inside a longer word
/// ("work" in "network") is left alone.
fn replace_labels(text: &str, labels: &[(String, String)]) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut out = text.to_string();
    for (label, id) in labels {
        if label.chars().count() < 2 {
            continue;
        }
        let mut res = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(label.as_str()) {
            let before = rest[..i].chars().next_back();
            let after = rest[i + label.len()..].chars().next();
            res.push_str(&rest[..i]);
            if before.is_some_and(word) || after.is_some_and(word) {
                res.push_str(label);
            } else {
                res.push('#');
                res.push_str(id);
            }
            rest = &rest[i + label.len()..];
        }
        res.push_str(rest);
        out = res;
    }
    out
}

/// Fields with a fixed vocabulary (enums, states): never label-replaced, so a label that equals
/// one of their values ("personal") cannot corrupt them. They still get the secret check.
const STRUCTURED_KEYS: &[&str] = &[
    "kind",
    "how",
    "state",
    "mode",
    "health",
    "subscription",
    "event",
    "level",
    "rank",
    "source",
    "stopReason",
    "errorKind",
    "code",
    "version",
    "protocol",
];

/// Token-like strings: sk-ant-..., long base64/hex runs, bearer values, emails.
fn scrub_secrets(out: &str) -> String {
    let mut cleaned = String::with_capacity(out.len());
    for word in out.split_inclusive(|c: char| c.is_whitespace() || c == '"' || c == ',') {
        let core = word.trim_end_matches(|c: char| c.is_whitespace() || c == '"' || c == ',');
        let tail = &word[core.len()..];
        // UUIDs (session ids) and plain hex digests (file hashes) are opaque ids, not secrets.
        let b = core.as_bytes();
        let uuid = (core.len() == 36
            && [8, 13, 18, 23].iter().all(|i| b[*i] == b'-')
            && core.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            || (core.len() == 64 && core.chars().all(|c| c.is_ascii_hexdigit()));
        let tokenish = core.starts_with("sk-ant-")
            || (!uuid && core.len() >= 32 && core.chars().all(|c| c.is_ascii_alphanumeric() || "-_=+/.".contains(c)))
            || (core.contains('@') && core.contains('.') && !core.starts_with('@'));
        if tokenish {
            cleaned.push_str("[redacted]");
        } else {
            cleaned.push_str(core);
        }
        cleaned.push_str(tail);
    }
    cleaned
}

pub fn scrub_value(v: &mut Value, labels: &[(String, String)]) {
    match v {
        Value::String(s) => *s = scrub(s, labels),
        Value::Array(a) => a.iter_mut().for_each(|x| scrub_value(x, labels)),
        Value::Object(o) => {
            o.retain(|k, _| {
                let k = k.to_ascii_lowercase();
                !(k.contains("token") || k.contains("secret") || k.contains("email") || k == "prompt" || k == "answer")
            });
            for (k, x) in o.iter_mut() {
                match x {
                    Value::String(s) if STRUCTURED_KEYS.contains(&k.as_str()) => *s = scrub_secrets(s),
                    _ => scrub_value(x, labels),
                }
            }
        }
        _ => {}
    }
}

pub struct Journal {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Journal {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Append one record. `durable` fsyncs (required before a transition write).
    pub fn append(
        &self,
        event: &str,
        mut data: Value,
        labels: &[(String, String)],
        durable: bool,
    ) -> std::io::Result<()> {
        scrub_value(&mut data, labels);
        let mut rec = json!({"ts": now_ms(), "event": event});
        if let (Some(r), Value::Object(d)) = (rec.as_object_mut(), data) {
            for (k, v) in d {
                if k != "ts" && k != "event" {
                    r.insert(k, v);
                }
            }
        }
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        f.write_all(format!("{rec}\n").as_bytes())?;
        if durable {
            f.sync_all()?;
        }
        Ok(())
    }

    pub fn read_all(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.path)
            .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default()
    }

    pub fn tail(&self, n: usize) -> Vec<Value> {
        let all = self.read_all();
        let start = all.len().saturating_sub(n);
        all[start..].to_vec()
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

pub struct Logger {
    dir: PathBuf,
    retention_days: u64,
    max_bytes: u64,
    /// Serializes writes; holds the day the logs were last pruned for (pruned on rollover).
    lock: Mutex<String>,
}

impl Logger {
    pub fn new(dir: PathBuf, retention_days: u64, max_mb: u64) -> Self {
        let l = Logger {
            dir,
            retention_days,
            max_bytes: max_mb * 1024 * 1024,
            lock: Mutex::new(day_stamp(now_ms())),
        };
        l.prune();
        l
    }

    fn file(&self) -> PathBuf {
        self.dir.join(format!("quayflow-{}.jsonl", day_stamp(now_ms())))
    }

    /// Delete log files older than the retention period.
    pub fn prune(&self) {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(self.retention_days * 86_400);
        if let Ok(d) = std::fs::read_dir(&self.dir) {
            for e in d.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("quayflow-") && name.ends_with(".jsonl") {
                    if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                        if m < cutoff {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
            }
        }
    }

    /// Log a sanitized record. Failure to log never interrupts work.
    pub fn log(&self, level: &str, event: &str, mut data: Value, labels: &[(String, String)]) {
        scrub_value(&mut data, labels);
        let rec = json!({"ts": now_ms(), "level": level, "event": event, "pid": std::process::id(), "data": data});
        let mut pruned_day = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        // A long-running engine prunes again at each day rollover, not only at start.
        let today = day_stamp(now_ms());
        if *pruned_day != today {
            self.prune();
            *pruned_day = today;
        }
        let path = self.file();
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > self.max_bytes {
            return;
        }
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(format!("{rec}\n").as_bytes());
        }
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        std::fs::read_to_string(self.file())
            .map(|t| {
                let lines: Vec<&str> = t.lines().collect();
                lines[lines.len().saturating_sub(n)..]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_replaced_as_whole_words_only_and_never_in_structured_fields() {
        let labels = vec![
            ("work".to_string(), "aaa".to_string()),
            ("personal".to_string(), "bbb".to_string()),
        ];
        assert_eq!(
            scrub("switched to work; network ok", &labels),
            "switched to #aaa; network ok"
        );
        assert_eq!(scrub("personal-ish personal", &labels), "personal-ish #bbb");
        let mut v = serde_json::json!({"kind": "personal", "label": "personal", "decision": "work at 90%"});
        scrub_value(&mut v, &labels);
        assert_eq!(v["kind"], "personal");
        assert_eq!(v["label"], "#bbb");
        assert_eq!(v["decision"], "#aaa at 90%");
    }

    #[test]
    fn scrub_removes_secrets_and_labels() {
        let labels = vec![("Work Org".to_string(), "abc123".to_string())];
        let s = scrub(
            "switch Work Org token sk-ant-oat01-xyz and me@example.com AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            &labels,
        );
        assert!(!s.contains("Work Org"));
        assert!(s.contains("#abc123"));
        assert!(!s.contains("sk-ant"));
        assert!(!s.contains("example.com"));
        assert!(!s.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
    }

    #[test]
    fn journal_never_writes_token_fields() {
        let d = tempfile::tempdir().unwrap();
        let j = Journal::new(d.path().join("j.jsonl"));
        j.append(
            "x",
            json!({"accessToken": "sk-ant-oat-1", "nested": {"refreshToken": "r", "ok": 1}, "reason": "uses sk-ant-ort-2", "email": "a@b.c"}),
            &[],
            true,
        )
        .unwrap();
        let text = std::fs::read_to_string(d.path().join("j.jsonl")).unwrap();
        assert!(
            !text.contains("sk-ant") && !text.contains("\"r\"") && !text.contains("a@b.c"),
            "{text}"
        );
        assert_eq!(j.read_all()[0]["nested"]["ok"], 1);
    }

    #[test]
    fn logger_caps_size() {
        let d = tempfile::tempdir().unwrap();
        let l = Logger::new(d.path().to_path_buf(), 7, 0);
        l.log("info", "a", json!({}), &[]);
        l.log("info", "b", json!({}), &[]);
        assert_eq!(l.tail(10).len(), 1, "stops writing past the cap");
    }

    #[test]
    fn logger_prunes_on_day_rollover() {
        let d = tempfile::tempdir().unwrap();
        let l = Logger::new(d.path().to_path_buf(), 7, 10);
        let old = d.path().join("quayflow-2000-01-01.jsonl");
        std::fs::write(
            &old, "{}
",
        )
        .unwrap();
        let f = std::fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400))
            .unwrap();
        drop(f);
        l.log("info", "same-day", json!({}), &[]);
        assert!(old.exists(), "no prune within the same day");
        // The engine kept running into the next day.
        *l.lock.lock().unwrap() = "yesterday".into();
        l.log("info", "next-day", json!({}), &[]);
        assert!(!old.exists(), "pruned on the day rollover");
        assert_eq!(l.tail(10).len(), 2);
    }
}
