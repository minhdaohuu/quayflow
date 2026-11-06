//! Lanes (FR-10): separate credential environments that link the shared Claude setup.
//!
//! A lane has its own login, `.claude.json`, `sessions\` and policy files. It links
//! `plugins\`, `skills\`, `agents\`, `commands\` and each project's `memory\` folder with
//! junctions, and `settings.json` / `CLAUDE.md` with hard links (no administrator rights).
//! If Claude replaces a linked file by rename the hard link breaks; [`check`] detects that
//! and re-links to the newer content.

use crate::env::Env;
use qf_model::EnvId;
use std::path::{Path, PathBuf};

pub const LINKED_DIRS: &[&str] = &["plugins", "skills", "agents", "commands"];
pub const LINKED_FILES: &[&str] = &["settings.json", "CLAUDE.md"];

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name != "default"
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn lane_env(lanes_dir: &Path, name: &str) -> Env {
    Env::at(EnvId(name.into()), lanes_dir.join(name))
}

#[derive(Debug, Clone)]
pub struct LinkStatus {
    pub item: String,
    pub ok: bool,
    pub detail: String,
}

#[cfg(windows)]
fn make_junction(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}

#[cfg(not(windows))]
fn make_junction(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn junction_target(link: &Path) -> Option<PathBuf> {
    junction::get_target(link).ok()
}

#[cfg(not(windows))]
fn junction_target(link: &Path) -> Option<PathBuf> {
    std::fs::read_link(link).ok()
}

fn is_link_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
        || junction_target(p).is_some()
}

/// Same file on disk (hard link check), by volume serial and file index.
#[cfg(windows)]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    fn id(p: &Path) -> Option<(u32, u32, u32)> {
        let f = std::fs::File::open(p).ok()?;
        // SAFETY: valid handle for the call; struct zero-initialized.
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) == 0 {
                return None;
            }
            Some((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
        }
    }
    matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(not(windows))]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.ino() == y.ino() && x.dev() == y.dev(),
        _ => false,
    }
}

fn memory_dirs(shared: &Env) -> Vec<String> {
    std::fs::read_dir(shared.projects_dir())
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().join("memory").is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Link one project's memory folder into the lane (also used when history moves in).
pub fn link_memory(shared: &Env, lane: &Env, project: &str) -> std::io::Result<()> {
    let target = shared.projects_dir().join(project).join("memory");
    if !target.is_dir() {
        std::fs::create_dir_all(&target)?;
    }
    let link = lane.projects_dir().join(project).join("memory");
    if link.exists() || is_link_dir(&link) {
        return Ok(());
    }
    std::fs::create_dir_all(link.parent().unwrap_or(&lane.projects_dir()))?;
    make_junction(&target, &link)
}

/// Create a lane. Its `.claude.json` starts as a copy of the shared one without the account,
/// so Claude does not run first-use setup; it has no login until one is switched in.
pub fn create(shared: &Env, lanes_dir: &Path, name: &str) -> Result<Env, String> {
    if !valid_name(name) {
        return Err("lane names are 1-32 characters: a-z, 0-9 and -".into());
    }
    let lane = lane_env(lanes_dir, name);
    if lane.config_dir.exists() {
        return Err(format!("lane \"{name}\" already exists"));
    }
    std::fs::create_dir_all(lane.config_dir.join("sessions")).map_err(|e| e.to_string())?;
    for d in LINKED_DIRS {
        let target = shared.config_dir.join(d);
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        make_junction(&target, &lane.config_dir.join(d)).map_err(|e| format!("{d}: {e}"))?;
    }
    for f in LINKED_FILES {
        let target = shared.config_dir.join(f);
        if target.is_file() {
            std::fs::hard_link(&target, lane.config_dir.join(f)).map_err(|e| format!("{f}: {e}"))?;
        }
    }
    for p in memory_dirs(shared) {
        link_memory(shared, &lane, &p).map_err(|e| format!("memory {p}: {e}"))?;
    }
    let mut cfg: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&shared.claude_json)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    cfg.remove("oauthAccount");
    qf_vault::atomic_write(
        &lane.claude_json,
        serde_json::to_string_pretty(&cfg).unwrap_or_default().as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    Ok(lane)
}

/// Overwrite an existing file's content in place (open, truncate, write), keeping its file
/// identity, so every other hard link to it (other lanes) still shares the new content.
/// A temp-and-rename write would give the shared file a new identity and break those links.
fn write_in_place(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).truncate(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Check (and repair) the lane's links.
pub fn check(shared: &Env, lane: &Env, repair: bool) -> Vec<LinkStatus> {
    let mut out = vec![];
    for d in LINKED_DIRS {
        let link = lane.config_dir.join(d);
        let target = shared.config_dir.join(d);
        let ok = junction_target(&link)
            .map(|t| {
                qf_model::normalize_path(&t.to_string_lossy()) == qf_model::normalize_path(&target.to_string_lossy())
            })
            .unwrap_or(false);
        let mut detail = if ok {
            "linked".to_string()
        } else {
            "not linked".to_string()
        };
        if !ok && repair && !link.exists() && make_junction(&target, &link).is_ok() {
            detail = "re-linked".into();
        }
        out.push(LinkStatus {
            item: d.to_string(),
            ok: ok || detail == "re-linked",
            detail,
        });
    }
    for f in LINKED_FILES {
        let link = lane.config_dir.join(f);
        let target = shared.config_dir.join(f);
        if !target.exists() && !link.exists() {
            out.push(LinkStatus {
                item: f.to_string(),
                ok: true,
                detail: "not present in the shared setup".into(),
            });
            continue;
        }
        if same_file(&link, &target) {
            out.push(LinkStatus {
                item: f.to_string(),
                ok: true,
                detail: "linked".into(),
            });
            continue;
        }
        if !target.exists() {
            // Deleted from the shared setup: the lane's leftover copy must not bring it back.
            let mut detail = "deleted from the shared setup; the lane still has its own copy".to_string();
            let mut ok = false;
            if repair {
                match std::fs::remove_file(&link) {
                    Ok(()) => {
                        ok = true;
                        detail = "lane copy removed (deleted from the shared setup)".into();
                    }
                    Err(e) => detail = format!("repair failed: {e}"),
                }
            }
            out.push(LinkStatus {
                item: f.to_string(),
                ok,
                detail,
            });
            continue;
        }
        // Broken link: the newer content wins, then link again.
        let mut detail = "link broken (file replaced)".to_string();
        let mut ok = false;
        if repair {
            let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let lane_newer = match (mtime(&link), mtime(&target)) {
                (Some(l), Some(t)) => l > t,
                _ => false,
            };
            let r = (|| -> std::io::Result<()> {
                if lane_newer {
                    let text = std::fs::read(&link)?;
                    write_in_place(&target, &text)?;
                }
                if link.exists() {
                    std::fs::remove_file(&link)?;
                }
                std::fs::hard_link(&target, &link)
            })();
            match r {
                Ok(()) => {
                    ok = true;
                    detail = if lane_newer {
                        "re-linked (lane copy was newer; kept)".into()
                    } else {
                        "re-linked".into()
                    };
                }
                Err(e) => detail = format!("repair failed: {e}"),
            }
        }
        out.push(LinkStatus {
            item: f.to_string(),
            ok,
            detail,
        });
    }
    out
}

/// Delete a lane. Links are removed first so shared folders are never touched.
pub fn delete(lane: &Env) -> std::io::Result<()> {
    for d in LINKED_DIRS {
        let p = lane.config_dir.join(d);
        if is_link_dir(&p) {
            std::fs::remove_dir(&p)?;
        }
    }
    if let Ok(projects) = std::fs::read_dir(lane.projects_dir()) {
        for p in projects.flatten() {
            let m = p.path().join("memory");
            if is_link_dir(&m) {
                std::fs::remove_dir(&m)?;
            }
        }
    }
    for f in LINKED_FILES {
        let _ = std::fs::remove_file(lane.config_dir.join(f));
    }
    std::fs::remove_dir_all(&lane.config_dir)?;
    let _ = std::fs::remove_dir(lane.legacy_lock());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_check_delete_keeps_shared_setup() {
        let d = tempfile::tempdir().unwrap();
        let shared = Env::default_at(d.path());
        std::fs::create_dir_all(shared.config_dir.join("skills").join("s1")).unwrap();
        std::fs::write(shared.config_dir.join("skills").join("s1").join("SKILL.md"), "x").unwrap();
        std::fs::write(shared.config_dir.join("settings.json"), "{\"a\":1}").unwrap();
        std::fs::create_dir_all(shared.projects_dir().join("C--p").join("memory")).unwrap();
        std::fs::write(
            &shared.claude_json,
            r#"{"oauthAccount":{"accountUuid":"u"},"hasCompletedOnboarding":true}"#,
        )
        .unwrap();
        let lanes = d.path().join("lanes");
        let lane = create(&shared, &lanes, "work").unwrap();
        assert!(lane.config_dir.join("skills").join("s1").join("SKILL.md").is_file());
        assert!(lane.projects_dir().join("C--p").join("memory").is_dir());
        let cfg = std::fs::read_to_string(&lane.claude_json).unwrap();
        assert!(!cfg.contains("oauthAccount") && cfg.contains("hasCompletedOnboarding"));
        // Writes through the link reach the shared file.
        std::fs::write(lane.config_dir.join("settings.json"), "{\"a\":2}").unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.config_dir.join("settings.json")).unwrap(),
            "{\"a\":2}"
        );
        assert!(check(&shared, &lane, false).iter().all(|s| s.ok));
        // Replace-by-rename breaks the hard link; check repairs it and keeps the newer lane content.
        std::thread::sleep(std::time::Duration::from_millis(20));
        qf_vault::atomic_write(&lane.config_dir.join("settings.json"), b"{\"a\":3}").unwrap();
        assert!(
            !check(&shared, &lane, false)
                .iter()
                .find(|s| s.item == "settings.json")
                .unwrap()
                .ok
        );
        assert!(check(&shared, &lane, true).iter().all(|s| s.ok));
        assert_eq!(
            std::fs::read_to_string(shared.config_dir.join("settings.json")).unwrap(),
            "{\"a\":3}"
        );
        delete(&lane).unwrap();
        assert!(!lane.config_dir.exists());
        assert!(
            shared.config_dir.join("skills").join("s1").join("SKILL.md").is_file(),
            "shared setup must survive"
        );
        assert!(shared.projects_dir().join("C--p").join("memory").is_dir());
        assert!(shared.config_dir.join("settings.json").is_file());
    }

    fn shared_with_settings(d: &Path) -> Env {
        let shared = Env::default_at(d);
        std::fs::create_dir_all(&shared.config_dir).unwrap();
        std::fs::write(shared.config_dir.join("settings.json"), "{\"a\":1}").unwrap();
        std::fs::write(&shared.claude_json, "{}").unwrap();
        shared
    }

    #[test]
    fn repair_keeps_other_lanes_hard_links() {
        let d = tempfile::tempdir().unwrap();
        let shared = shared_with_settings(d.path());
        let lanes = d.path().join("lanes");
        let a = create(&shared, &lanes, "a").unwrap();
        let b = create(&shared, &lanes, "b").unwrap();
        let shared_file = shared.config_dir.join("settings.json");
        let (fa, fb) = (a.config_dir.join("settings.json"), b.config_dir.join("settings.json"));
        assert!(same_file(&fa, &shared_file) && same_file(&fb, &shared_file));
        // Claude in lane a replaces its copy by rename: a's link breaks, a's copy is newer.
        std::thread::sleep(std::time::Duration::from_millis(20));
        qf_vault::atomic_write(&fa, b"{\"a\":2}").unwrap();
        assert!(check(&shared, &a, true).iter().all(|s| s.ok));
        assert_eq!(std::fs::read_to_string(&shared_file).unwrap(), "{\"a\":2}");
        // Lane b's hard link survived the repair and shares the new content.
        assert!(same_file(&fb, &shared_file), "b's hard link must survive a's repair");
        assert_eq!(std::fs::read_to_string(&fb).unwrap(), "{\"a\":2}");
        assert!(check(&shared, &b, false).iter().all(|s| s.ok));
        std::fs::write(&fb, "{\"a\":3}").unwrap();
        assert_eq!(std::fs::read_to_string(&fa).unwrap(), "{\"a\":3}");
    }

    #[test]
    fn file_deleted_from_shared_setup_is_not_recreated() {
        let d = tempfile::tempdir().unwrap();
        let shared = shared_with_settings(d.path());
        let lane = create(&shared, &d.path().join("lanes"), "a").unwrap();
        let shared_file = shared.config_dir.join("settings.json");
        std::fs::remove_file(&shared_file).unwrap();
        let st = |r: Vec<LinkStatus>| r.into_iter().find(|s| s.item == "settings.json").unwrap();
        assert!(!st(check(&shared, &lane, false)).ok);
        assert!(st(check(&shared, &lane, true)).ok);
        assert!(!shared_file.exists(), "the deleted shared file must not come back");
        assert!(!lane.config_dir.join("settings.json").exists());
        assert!(st(check(&shared, &lane, false)).ok);
    }

    #[test]
    fn names() {
        assert!(valid_name("work-2"));
        assert!(!valid_name("default"));
        assert!(!valid_name("Work"));
        assert!(!valid_name("..\\x"));
    }
}
