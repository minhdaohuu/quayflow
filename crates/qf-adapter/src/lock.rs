//! Claude Code's credential lock protocol (proper-lockfile directory locks).
//!
//! `mkdir` is the mutex. Holders touch the directory's mtime periodically; a lock older
//! than its stale age belongs to a dead holder and may be removed. Quayflow takes the
//! refresh lock, then the legacy lock, then the config lock; never removes a lock younger
//! than its stale age; and gives up after a bounded wait with nothing changed.

use crate::env::Env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    stop: Arc<AtomicBool>,
    toucher: Option<std::thread::JoinHandle<()>>,
}

impl HeldLocks {
    pub fn names(&self) -> usize {
        self.paths.len()
    }
}

impl Drop for HeldLocks {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.toucher.take() {
            let _ = t.join();
        }
        for p in self.paths.iter().rev() {
            let _ = std::fs::remove_dir(p);
        }
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

/// Acquire one lock, waiting up to `wait`.
pub fn acquire_one(spec: &LockSpec, wait: Duration) -> Result<(), LockError> {
    let start = Instant::now();
    let mut rng = (std::process::id() as u64).wrapping_mul(6364136223846793005);
    loop {
        match std::fs::create_dir(&spec.path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Parent missing (fresh environment): create it, then retry.
                if let Some(p) = spec.path.parent() {
                    std::fs::create_dir_all(p).map_err(LockError::Io)?;
                }
                continue;
            }
            Err(e) if qf_vault::is_sharing_violation(&e) => {}
            Err(e) => return Err(LockError::Io(e)),
        }
        match age(&spec.path) {
            // Released between mkdir and stat.
            None => continue,
            // Only a holder silent past the stale age is dead.
            Some(a) if a > spec.stale => {
                let _ = std::fs::remove_dir(&spec.path);
                continue;
            }
            Some(_) => {}
        }
        if start.elapsed() > wait {
            return Err(LockError::Held {
                name: spec.name,
                waited_ms: start.elapsed().as_millis() as u64,
            });
        }
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        std::thread::sleep(Duration::from_millis(250 + rng % 250));
    }
}

/// Take the locks in Claude's order. On failure, release what was taken.
pub fn acquire(specs: &[LockSpec], wait_each: Duration) -> Result<HeldLocks, LockError> {
    let mut held: Vec<PathBuf> = vec![];
    for s in specs {
        if let Err(e) = acquire_one(s, wait_each) {
            for p in held.iter().rev() {
                let _ = std::fs::remove_dir(p);
            }
            return Err(e);
        }
        held.push(s.path.clone());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (paths, flag) = (held.clone(), stop.clone());
    let toucher = std::thread::spawn(move || {
        let mut last = Instant::now();
        while !flag.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            if last.elapsed() >= Duration::from_secs(2) {
                for p in &paths {
                    let _ = touch_dir(p);
                }
                last = Instant::now();
            }
        }
    });
    Ok(HeldLocks {
        paths: held,
        stop,
        toucher: Some(toucher),
    })
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
