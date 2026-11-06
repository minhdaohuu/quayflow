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

fn io(e: std::io::Error) -> HistoryError {
    HistoryError::Io(e.to_string())
}

/// Copy a file or folder tree, verifying each copied file against the hash of the source
/// bytes that were actually written (read once). Returns the number of files copied.
fn copy_verified(src: &Path, dst: &Path, rel: &Path) -> Result<usize, HistoryError> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(io)?;
        let mut n = 0;
        for e in std::fs::read_dir(src).map_err(io)?.flatten() {
            n += copy_verified(&e.path(), &dst.join(e.file_name()), &rel.join(e.file_name()))?;
        }
        Ok(n)
    } else {
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p).map_err(io)?;
        }
        let bytes = std::fs::read(src).map_err(io)?;
        let want = Sha256::digest(&bytes);
        std::fs::write(dst, &bytes).map_err(io)?;
        fail_point("copy")?;
        let got = Sha256::digest(std::fs::read(dst).map_err(io)?);
        if got != want {
            return Err(HistoryError::Integrity(rel.display().to_string()));
        }
        Ok(1)
    }
}

fn remove_path(p: &Path) {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let _ = std::fs::remove_dir_all(p);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(p);
        }
        Err(_) => {}
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// Test hook: force a failure at a named step of the copy.
#[cfg(test)]
fn fail_point(step: &'static str) -> Result<(), HistoryError> {
    if FAIL_AT.with(|f| f.get()) == Some(step) {
        return Err(HistoryError::Io(format!("forced failure at {step}")));
    }
    Ok(())
}

#[cfg(not(test))]
#[inline(always)]
fn fail_point(_step: &'static str) -> Result<(), HistoryError> {
    Ok(())
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
    // 1. Copy into staging names beside the destination and verify every file. The
    //    destination is untouched until all of it is in place.
    std::fs::create_dir_all(&dst_project).map_err(io)?;
    let tag = format!("{}-{}", std::process::id(), qf_model::time::now_ms());
    let mut staged: Vec<(PathBuf, PathBuf, &String)> = vec![];
    let mut count = 0;
    let mut stage = || -> Result<(), HistoryError> {
        for name in &items {
            let staging = dst_project.join(format!(".{name}.quayflow-staging-{tag}"));
            staged.push((staging.clone(), dst_project.join(name), name));
            count += copy_verified(&src_project.join(name), &staging, Path::new(name))?;
        }
        Ok(())
    };
    if let Err(e) = stage() {
        for (staging, _, _) in &staged {
            remove_path(staging);
        }
        return Err(e);
    }

    // 2. Swap in: move each existing item to the backups, then the staged copy into place.
    //    On any failure, undo what was swapped so the destination keeps its history.
    let mut moved: Vec<(PathBuf, PathBuf)> = vec![];
    let mut placed: Vec<PathBuf> = vec![];
    let mut swap = || -> Result<(), HistoryError> {
        for (staging, dst, name) in &staged {
            if dst.exists() {
                std::fs::create_dir_all(backups_dir).map_err(io)?;
                let backup = backups_dir.join(format!("{}-{}", qf_model::time::now_ms(), name));
                std::fs::rename(dst, &backup).map_err(io)?;
                moved.push((dst.clone(), backup));
            }
            std::fs::rename(staging, dst).map_err(io)?;
            placed.push(dst.clone());
            fail_point("swap")?;
        }
        Ok(())
    };
    if let Err(e) = swap() {
        for dst in placed.iter().rev() {
            remove_path(dst);
        }
        for (dst, backup) in moved.iter().rev() {
            let _ = std::fs::rename(backup, dst);
        }
        for (staging, _, _) in &staged {
            remove_path(staging);
        }
        return Err(e);
    }
    backups.extend(moved.into_iter().map(|(_, b)| b));
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

    /// Every file under `dir` with its content, for before/after comparison.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut v: Vec<_> = list_files(dir)
            .into_iter()
            .map(|f| (f.strip_prefix(dir).unwrap().to_path_buf(), std::fs::read(&f).unwrap()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn failed_copy_leaves_destination_unchanged() {
        let (d, a, b) = envs();
        let bk = d.path().join("bk");
        copy_session(&a, &b, SID, &bk).unwrap();
        let dst_project = b.projects_dir().join("C--proj");
        let before = snapshot(&dst_project);
        // The source moves on; then the copy fails at each step.
        let p = a.projects_dir().join("C--proj");
        std::fs::write(p.join(format!("{SID}.jsonl")), "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n").unwrap();
        std::fs::write(p.join(SID).join("tool-results").join("r2.txt"), "more").unwrap();
        for step in ["copy", "swap"] {
            FAIL_AT.with(|f| f.set(Some(step)));
            let r = copy_session(&a, &b, SID, &bk);
            FAIL_AT.with(|f| f.set(None));
            assert!(matches!(r, Err(HistoryError::Io(_))), "{step}");
            assert_eq!(
                snapshot(&dst_project),
                before,
                "destination changed after a failure at {step}"
            );
            let backups_left = std::fs::read_dir(&bk).map(|r| r.count()).unwrap_or(0);
            assert_eq!(backups_left, 0, "backups restored after a failure at {step}");
        }
        // Without a failure it advances.
        let r = copy_session(&a, &b, SID, &bk).unwrap();
        assert_eq!((r.file_count, r.transcript_rows, r.backups.len()), (3, 3, 2));
        assert_eq!(snapshot(&dst_project), snapshot(&p));
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
