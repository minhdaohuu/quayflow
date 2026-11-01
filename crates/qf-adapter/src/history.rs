//! Full-history copy for Open/Replace (FR-17, FR-18): the session transcript and its side
//! folder, with per-file SHA-256 checks. A newer or diverged destination is never
//! overwritten; an older copy of the same history is advanced and kept as a backup;
//! the source is never modified.

use crate::env::Env;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum HistoryError {
    NotFound,
    /// The destination holds history that is not an older state of the source.
    Diverged(String),
    Io(String),
    Integrity(String),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::NotFound => write!(f, "session history not found in the source environment"),
            HistoryError::Diverged(file) => write!(
                f,
                "destination history is newer or diverged ({file}); refusing to overwrite it"
            ),
            HistoryError::Io(m) => write!(f, "history copy failed: {m}"),
            HistoryError::Integrity(file) => write!(f, "hash mismatch after copying {file}"),
        }
    }
}

impl std::error::Error for HistoryError {}

#[derive(Debug, Clone)]
pub struct CopyResult {
    pub project: String,
    pub file_count: usize,
    pub transcript_rows: usize,
    pub backups: Vec<PathBuf>,
    pub already_current: bool,
}

pub fn valid_session_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Project folder holding the session transcript.
pub fn find_project(env: &Env, session_id: &str) -> Option<String> {
    let dir = std::fs::read_dir(env.projects_dir()).ok()?;
    dir.flatten()
        .find(|e| e.path().join(format!("{session_id}.jsonl")).is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
}

fn sha(path: &Path) -> Result<String, HistoryError> {
    let bytes = std::fs::read(path).map_err(|e| HistoryError::Io(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn list_files(p: &Path) -> Vec<PathBuf> {
    if p.is_dir() {
        std::fs::read_dir(p)
            .map(|d| d.flatten().flat_map(|e| list_files(&e.path())).collect())
            .unwrap_or_default()
    } else if p.is_file() {
        vec![p.to_path_buf()]
    } else {
        vec![]
    }
}

fn is_prefix(older: &Path, newer: &Path) -> bool {
    match (std::fs::read(older), std::fs::read(newer)) {
        (Ok(a), Ok(b)) => a.len() <= b.len() && b[..a.len()] == a[..],
        _ => false,
    }
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// Copy a session's full retained history from one environment to another.
pub fn copy_session(from: &Env, to: &Env, session_id: &str, backups_dir: &Path) -> Result<CopyResult, HistoryError> {
    if !valid_session_id(session_id) {
        return Err(HistoryError::NotFound);
    }
    let project = find_project(from, session_id).ok_or(HistoryError::NotFound)?;
    let src_project = from.projects_dir().join(&project);
    let dst_project = to.projects_dir().join(&project);
    let items: Vec<String> = [format!("{session_id}.jsonl"), session_id.to_string()]
        .into_iter()
        .filter(|n| src_project.join(n).exists())
        .collect();

    // Destination already holds this session: it must be an older state of the same history.
    let mut backups = vec![];
    let existing: Vec<&String> = items.iter().filter(|n| dst_project.join(n).exists()).collect();
    let mut identical = !existing.is_empty() && existing.len() == items.len();
    for name in &existing {
        let dst = dst_project.join(name);
        for file in list_files(&dst) {
            let rel = file.strip_prefix(&dst_project).unwrap_or(&file).to_path_buf();
            let src = src_project.join(&rel);
            if !src.is_file() || !is_prefix(&file, &src) {
                return Err(HistoryError::Diverged(rel.display().to_string()));
            }
            if sha(&file)? != sha(&src)? {
                identical = false;
            }
        }
        if list_files(&src_project.join(name)).len() != list_files(&dst).len() {
            identical = false;
        }
    }
    let rows = std::fs::read_to_string(src_project.join(format!("{session_id}.jsonl")))
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    if identical {
        let count = items.iter().map(|n| list_files(&src_project.join(n)).len()).sum();
        return Ok(CopyResult {
            project,
            file_count: count,
            transcript_rows: rows,
            backups,
            already_current: true,
        });
    }
    for name in existing {
        let dst = dst_project.join(name);
        let backup = backups_dir.join(format!("{}-{}", qf_model::time::now_ms(), name));
        std::fs::create_dir_all(backups_dir).map_err(|e| HistoryError::Io(e.to_string()))?;
        std::fs::rename(&dst, &backup).map_err(|e| HistoryError::Io(e.to_string()))?;
        backups.push(backup);
    }

    std::fs::create_dir_all(&dst_project).map_err(|e| HistoryError::Io(e.to_string()))?;
    let mut count = 0;
    for name in &items {
        let src = src_project.join(name);
        let dst = dst_project.join(name);
        copy_tree(&src, &dst).map_err(|e| HistoryError::Io(e.to_string()))?;
        for file in list_files(&src) {
            let rel = file.strip_prefix(&src).unwrap_or(&file).to_path_buf();
            let copy = if rel.as_os_str().is_empty() {
                dst.clone()
            } else {
                dst.join(&rel)
            };
            if sha(&file)? != sha(&copy)? {
                return Err(HistoryError::Integrity(rel.display().to_string()));
            }
            count += 1;
        }
    }
    Ok(CopyResult {
        project,
        file_count: count,
        transcript_rows: rows,
        backups,
        already_current: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SID: &str = "49baafcc-0000-4000-8000-000000000001";

    fn envs() -> (tempfile::TempDir, Env, Env) {
        let d = tempfile::tempdir().unwrap();
        let a = Env::default_at(&d.path().join("a"));
        let b = Env::at(qf_model::EnvId("lane".into()), d.path().join("b"));
        let p = a.projects_dir().join("C--proj");
        std::fs::create_dir_all(p.join(SID).join("tool-results")).unwrap();
        std::fs::write(p.join(format!("{SID}.jsonl")), "{\"a\":1}\n{\"a\":2}\n").unwrap();
        std::fs::write(p.join(SID).join("tool-results").join("r1.txt"), "result").unwrap();
        (d, a, b)
    }

    #[test]
    fn copies_full_history_and_keeps_source() {
        let (d, a, b) = envs();
        let r = copy_session(&a, &b, SID, &d.path().join("bk")).unwrap();
        assert_eq!(r.file_count, 2);
        assert_eq!(r.transcript_rows, 2);
        assert!(b
            .projects_dir()
            .join("C--proj")
            .join(SID)
            .join("tool-results")
            .join("r1.txt")
            .is_file());
        assert!(a.projects_dir().join("C--proj").join(format!("{SID}.jsonl")).is_file());
        // Running again is a no-op.
        assert!(copy_session(&a, &b, SID, &d.path().join("bk")).unwrap().already_current);
    }

    #[test]
    fn advances_older_copy_with_backup() {
        let (d, a, b) = envs();
        copy_session(&a, &b, SID, &d.path().join("bk")).unwrap();
        let src = a.projects_dir().join("C--proj").join(format!("{SID}.jsonl"));
        std::fs::write(&src, "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n").unwrap();
        let r = copy_session(&a, &b, SID, &d.path().join("bk")).unwrap();
        assert!(!r.backups.is_empty());
        assert_eq!(r.transcript_rows, 3);
    }

    #[test]
    fn refuses_diverged_destination() {
        let (d, a, b) = envs();
        copy_session(&a, &b, SID, &d.path().join("bk")).unwrap();
        let dst = b.projects_dir().join("C--proj").join(format!("{SID}.jsonl"));
        std::fs::write(&dst, "{\"a\":1}\n{\"b\":\"newer elsewhere\"}\n").unwrap();
        assert!(matches!(
            copy_session(&a, &b, SID, &d.path().join("bk")),
            Err(HistoryError::Diverged(_))
        ));
        assert!(std::fs::read_to_string(&dst).unwrap().contains("newer elsewhere"));
    }
}
