//! Claude Code's credential lock protocol (proper-lockfile directory locks).
//!
//! `mkdir` is the mutex. Holders touch the directory's mtime periodically; a lock older
//! than its stale age belongs to a dead holder and may be removed. Quayflow takes the
//! refresh lock, then the legacy lock, then the config lock; never removes a lock younger
//! than its stale age; and gives up after a bounded wait with nothing changed.

use crate::env::Env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone)]
pub struct LockSpec {
    pub name: &'static str,
    pub path: PathBuf,
    pub stale: Duration,
}

pub fn specs(env: &Env) -> Vec<LockSpec> {
    vec![
        LockSpec {
            name: "refresh",
            path: env.refresh_lock(),
            stale: Duration::from_secs(60),
        },
        LockSpec {
            name: "legacy",
            path: env.legacy_lock(),
            stale: Duration::from_secs(60),
        },
        LockSpec {
            name: "config",
            path: env.config_lock(),
            stale: Duration::from_secs(10),
        },
    ]
}

#[derive(Debug)]
pub enum LockError {
    /// Held by a live holder past the bounded wait. Nothing was changed.
    Held {
        name: &'static str,
        waited_ms: u64,
    },
    Io(std::io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held { name, waited_ms } => write!(
                f,
                "Claude's {name} lock stayed held for {} s (Claude is refreshing or saving); the login was not changed",
                waited_ms / 1000
            ),
            LockError::Io(e) => write!(f, "lock error: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Locks held in order; released in reverse order on drop. A background thread
/// touches them every 2 s so Claude never sees them as stale.
pub struct HeldLocks {
    paths: Vec<PathBuf>,
    toucher: Toucher,
}

impl HeldLocks {
    pub fn names(&self) -> usize {
        self.paths.len()
    }
}

impl Drop for HeldLocks {
    fn drop(&mut self) {
        // Stop touching before removing, so no touch races the release.
        self.toucher.stop();
        for p in self.paths.iter().rev() {
            let _ = std::fs::remove_dir(p);
        }
    }
}

const TOUCH_EVERY: Duration = Duration::from_secs(2);

/// Background thread that touches every lock added to it every 2 s. It starts before the
/// first lock is taken, so a lock already held stays fresh while the next one is awaited.
struct Toucher {
    paths: Arc<Mutex<Vec<PathBuf>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Toucher {
    fn start() -> Self {
        let paths: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (list, flag) = (paths.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut last = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                if last.elapsed() >= TOUCH_EVERY {
                    let held = list.lock().map(|g| g.clone()).unwrap_or_default();
                    for p in &held {
                        let _ = touch_dir(p);
                    }
                    last = Instant::now();
                }
            }
        });
        Toucher {
            paths,
            stop,
            thread: Some(thread),
        }
    }

    fn add(&self, path: PathBuf) {
        if let Ok(mut g) = self.paths.lock() {
            g.push(path);
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Toucher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn age(path: &Path) -> Option<Duration> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(SystemTime::now().duration_since(m).unwrap_or_default())
}

pub fn touch_dir(path: &Path) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS opens a directory handle.
        opts.custom_flags(0x0200_0000);
    }
    let f = opts.open(path)?;
    f.set_modified(SystemTime::now())
}

/// Remove a lock directory whose holder is dead, without ever deleting a live lock.
///
/// The age is re-checked right before acting, and the directory is first renamed to a
/// unique name: if another waiter removed the stale lock and a new holder created a fresh
/// one in between, the renamed directory is that fresh lock, and it is put back instead of
/// deleted. Returns whether the lock path was freed.
fn remove_stale(spec: &LockSpec) -> bool {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = &spec.path;
    // Only a directory is a lock we may remove; anything else at the path is left alone.
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return false;
    }
    if !age(path).is_some_and(|a| a > spec.stale) {
        return false;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".quayflow-stale-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let aside = path.with_file_name(name);
    if std::fs::rename(path, &aside).is_err() {
        return false;
    }
    // Renaming does not change the directory's own modification time.
    if age(&aside).is_some_and(|a| a > spec.stale) && std::fs::remove_dir(&aside).is_ok() {
        return true;
    }
    // A fresh lock was moved, or the stale one cannot be deleted: put it back.
    put_back(&aside, path);
    false
}

/// Return a directory moved aside by [`remove_stale`] to the lock path. If the path was
/// taken by a new lock in the meantime, the moved one cannot go back (that would replace or
/// merge with the new holder's lock); it is removed when empty instead of being left
/// stranded under its temporary name, where no holder or waiter would ever find it.
fn put_back(aside: &Path, path: &Path) {
    for attempt in 0..3 {
        if std::fs::rename(aside, path).is_ok() {
            return;
        }
        if std::fs::symlink_metadata(path).is_ok() {
            // `remove_dir` only removes an empty directory: a lock directory, never content.
            let _ = std::fs::remove_dir(aside);
            return;
        }
        std::thread::sleep(Duration::from_millis(10 * (attempt + 1)));
    }
}

/// Acquire one lock, waiting up to `wait`. Every path through the loop honors the deadline
/// and sleeps, so a lock that can be neither taken nor removed never spins.
pub fn acquire_one(spec: &LockSpec, wait: Duration) -> Result<(), LockError> {
    let start = Instant::now();
    let mut rng = (std::process::id() as u64).wrapping_mul(6364136223846793005) | 1;
    let mut made_parent = false;
    loop {
        // A short pause when the next attempt is likely to win; otherwise a jittered one.
        let mut quick = false;
        match std::fs::create_dir(&spec.path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Parent missing (fresh environment): create it, then retry at once (once).
                if let Some(p) = spec.path.parent() {
                    std::fs::create_dir_all(p).map_err(LockError::Io)?;
                }
                if !made_parent {
                    made_parent = true;
                    continue;
                }
                quick = true;
            }
            Err(e) if qf_vault::is_sharing_violation(&e) => {}
            Err(e) => return Err(LockError::Io(e)),
        }
        if !quick {
            match age(&spec.path) {
                // Released between mkdir and stat: retry soon.
                None => quick = true,
                // Only a holder silent past the stale age is dead. If it cannot be removed,
                // keep waiting like for a live holder.
                Some(a) if a > spec.stale => quick = remove_stale(spec),
                Some(_) => {}
            }
        }
        let elapsed = start.elapsed();
        if elapsed >= wait {
            return Err(LockError::Held {
                name: spec.name,
                waited_ms: elapsed.as_millis() as u64,
            });
        }
        let pause = if quick {
            Duration::from_millis(25)
        } else {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            Duration::from_millis(250 + rng % 250)
        };
        std::thread::sleep(pause.min(wait - elapsed).max(Duration::from_millis(1)));
    }
}

/// Take the locks in Claude's order. On failure, release what was taken.
/// Each lock is touched from the moment it is taken, also while the next one is awaited.
pub fn acquire(specs: &[LockSpec], wait_each: Duration) -> Result<HeldLocks, LockError> {
    let mut toucher = Toucher::start();
    let mut held: Vec<PathBuf> = vec![];
    for s in specs {
        if let Err(e) = acquire_one(s, wait_each) {
            toucher.stop();
            for p in held.iter().rev() {
                let _ = std::fs::remove_dir(p);
            }
            return Err(e);
        }
        held.push(s.path.clone());
        toucher.add(s.path.clone());
    }
    Ok(HeldLocks { paths: held, toucher })
}

pub fn acquire_env(env: &Env, wait_each: Duration) -> Result<HeldLocks, LockError> {
    acquire(&specs(env), wait_each)
}

/// Refresh and legacy locks only: enough to read a consistent login.
pub fn acquire_read(env: &Env, wait_each: Duration) -> Result<HeldLocks, LockError> {
    acquire(&specs(env)[..2], wait_each)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dir: &Path) -> Env {
        Env::default_at(dir)
    }

    #[test]
    fn acquire_and_release() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        std::fs::create_dir_all(&e.config_dir).unwrap();
        {
            let held = acquire_env(&e, Duration::from_secs(1)).unwrap();
            assert_eq!(held.names(), 3);
            assert!(e.refresh_lock().exists() && e.legacy_lock().exists() && e.config_lock().exists());
        }
        assert!(!e.refresh_lock().exists() && !e.legacy_lock().exists() && !e.config_lock().exists());
    }

    #[test]
    fn live_lock_is_never_stolen_and_nothing_is_left_held() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        std::fs::create_dir_all(&e.config_dir).unwrap();
        std::fs::create_dir(e.legacy_lock()).unwrap(); // a live holder of the legacy lock
        let r = acquire_env(&e, Duration::from_millis(600));
        assert!(matches!(r, Err(LockError::Held { name: "legacy", .. })));
        assert!(e.legacy_lock().exists(), "the live lock must stay");
        assert!(!e.refresh_lock().exists(), "the refresh lock we took must be released");
    }

    #[test]
    fn stale_lock_is_taken_over() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        std::fs::create_dir_all(&e.config_dir).unwrap();
        std::fs::create_dir(e.config_lock()).unwrap();
        let old = SystemTime::now() - Duration::from_secs(30);
        let mut o = std::fs::OpenOptions::new();
        o.write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.custom_flags(0x0200_0000);
        }
        o.open(e.config_lock()).unwrap().set_modified(old).unwrap();
        let held = acquire_env(&e, Duration::from_millis(600)).unwrap();
        drop(held);
    }

    fn set_mtime(p: &Path, t: SystemTime) {
        let mut o = std::fs::OpenOptions::new();
        o.write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.custom_flags(0x0200_0000);
        }
        o.open(p).unwrap().set_modified(t).unwrap();
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("quayflow-stale"))
            .collect()
    }

    #[test]
    fn unremovable_stale_lock_waits_until_the_deadline() {
        let d = tempfile::tempdir().unwrap();
        let spec = LockSpec {
            name: "config",
            path: d.path().join("x.lock"),
            stale: Duration::from_secs(10),
        };
        // A FILE at the lock path, far past the stale age: it cannot be removed as a lock.
        std::fs::write(&spec.path, "not a lock").unwrap();
        set_mtime(&spec.path, SystemTime::now() - Duration::from_secs(600));
        let t = Instant::now();
        let r = acquire_one(&spec, Duration::from_millis(700));
        assert!(matches!(r, Err(LockError::Held { name: "config", .. })));
        assert!(t.elapsed() < Duration::from_secs(3), "must give up at the deadline");
        assert!(spec.path.is_file(), "the file must be left alone");
    }

    #[test]
    fn stale_removal_never_deletes_a_fresh_or_unremovable_lock() {
        let d = tempfile::tempdir().unwrap();
        let spec = LockSpec {
            name: "config",
            path: d.path().join("x.lock"),
            stale: Duration::from_secs(10),
        };
        // Fresh: kept.
        std::fs::create_dir(&spec.path).unwrap();
        assert!(!remove_stale(&spec));
        assert!(spec.path.is_dir());
        // Stale but not empty: cannot be deleted, so it is put back where it was.
        std::fs::write(spec.path.join("f"), "x").unwrap();
        set_mtime(&spec.path, SystemTime::now() - Duration::from_secs(600));
        assert!(!remove_stale(&spec));
        assert!(spec.path.join("f").is_file());
        assert!(leftovers(d.path()).is_empty());
        // Stale and empty: removed, nothing left aside.
        std::fs::remove_file(spec.path.join("f")).unwrap();
        set_mtime(&spec.path, SystemTime::now() - Duration::from_secs(600));
        assert!(remove_stale(&spec));
        assert!(!spec.path.exists());
        assert!(leftovers(d.path()).is_empty());
    }

    #[test]
    fn moved_lock_is_not_stranded_when_a_new_lock_took_the_path() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("x.lock");
        let aside = d.path().join("x.lock.quayflow-stale-1-0");
        // A fresh lock was moved aside, and a new holder created the lock meanwhile.
        std::fs::create_dir(&aside).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("marker"), "new holder").unwrap();
        put_back(&aside, &path);
        assert!(leftovers(d.path()).is_empty(), "nothing left under the temporary name");
        assert!(path.join("marker").is_file(), "the new lock is untouched");
        // With the path free, the moved lock goes back.
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::create_dir(&aside).unwrap();
        put_back(&aside, &path);
        assert!(path.is_dir() && !aside.exists());
    }

    #[test]
    fn held_lock_is_touched_while_waiting_for_the_next() {
        let d = tempfile::tempdir().unwrap();
        let specs = vec![
            LockSpec {
                name: "refresh",
                path: d.path().join("a.lock"),
                stale: Duration::from_secs(60),
            },
            LockSpec {
                name: "legacy",
                path: d.path().join("b.lock"),
                stale: Duration::from_secs(60),
            },
        ];
        std::fs::create_dir(&specs[1].path).unwrap(); // a live holder of the second lock
        let a = specs[0].path.clone();
        let watcher = std::thread::spawn(move || {
            // Once the first lock is taken, make it look old; the toucher must refresh it.
            let t = Instant::now();
            while !a.exists() && t.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(10));
            }
            set_mtime(&a, SystemTime::now() - Duration::from_secs(3600));
            std::thread::sleep(Duration::from_millis(2600));
            age(&a)
        });
        let r = acquire(&specs, Duration::from_millis(3500));
        assert!(matches!(r, Err(LockError::Held { name: "legacy", .. })));
        let seen = watcher.join().unwrap().expect("first lock held during the wait");
        assert!(
            seen < Duration::from_secs(60),
            "first lock not touched while waiting: {seen:?}"
        );
        assert!(!specs[0].path.exists(), "released on failure");
    }

    #[test]
    fn contention_between_threads_serializes() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        std::fs::create_dir_all(&e.config_dir).unwrap();
        let counter = Arc::new(std::sync::Mutex::new((0u32, 0u32)));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (e, c) = (e.clone(), counter.clone());
                std::thread::spawn(move || {
                    let _held = acquire_env(&e, Duration::from_secs(20)).unwrap();
                    {
                        let mut g = c.lock().unwrap();
                        g.0 += 1;
                        g.1 = g.1.max(g.0);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                    c.lock().unwrap().0 -= 1;
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(counter.lock().unwrap().1, 1, "never two holders at once");
    }
}
