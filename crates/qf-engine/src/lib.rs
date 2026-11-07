//! Quayflow engine: one per Windows user (FR-19), loopback API (FR-20), automation loop.

pub mod accounts;
pub mod core;
pub mod http;
pub mod inspect;
pub mod journal;
pub mod lanes_ops;
pub mod paths;
pub mod reconcile;
pub mod recovery;
pub mod report;
pub mod transition;
pub mod views;

pub use crate::core::{Engine, Shared};
pub use paths::{DataDir, EngineConn};

use qf_model::time::now_ms;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The single-instance lock, held for the engine's lifetime. Dropping it releases the lock.
pub struct InstanceLock {
    _file: std::fs::File,
}

/// Single-instance lock: `engine.lock` held open without write sharing for the engine's
/// lifetime, holding our PID (informational). Another engine cannot open it while we run;
/// the OS closes the handle when an engine crashes, so the next engine simply takes it.
/// Returns `None` when another engine runs.
#[cfg(windows)]
pub fn take_instance_lock(dd: &DataDir) -> Result<Option<InstanceLock>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .open(dd.engine_lock());
    match opened {
        Ok(mut f) => {
            f.set_len(0).map_err(|e| e.to_string())?;
            f.write_all(std::process::id().to_string().as_bytes())
                .map_err(|e| e.to_string())?;
            Ok(Some(InstanceLock { _file: f }))
        }
        // ERROR_SHARING_VIOLATION 32, ERROR_LOCK_VIOLATION 33: another engine holds it.
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Single-instance lock (Unix): an advisory `flock` on `engine.lock`, held for the engine's
/// lifetime. The kernel drops it when the process ends, crashed or not.
#[cfg(not(windows))]
pub fn take_instance_lock(dd: &DataDir) -> Result<Option<InstanceLock>, String> {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dd.engine_lock())
        .map_err(|e| e.to_string())?;
    match f.try_lock() {
        Ok(()) => {
            f.set_len(0).map_err(|e| e.to_string())?;
            f.write_all(std::process::id().to_string().as_bytes())
                .map_err(|e| e.to_string())?;
            Ok(Some(InstanceLock { _file: f }))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
    }
}

pub fn release_instance_lock(dd: &DataDir, lock: InstanceLock) {
    if EngineConn::read(dd).is_some_and(|c| c.pid == std::process::id()) {
        let _ = std::fs::remove_file(dd.engine_json());
    }
    // The lock file itself stays: it is only a lock while an engine holds it open.
    drop(lock);
}

/// Run the engine until stopped. Returns Ok(false) when another engine already runs.
pub fn serve(dd: DataDir, default_env: qf_adapter::Env, tick: Duration) -> Result<bool, String> {
    dd.ensure().map_err(|e| e.to_string())?;
    let Some(lock) = take_instance_lock(&dd)? else {
        return Ok(false);
    };
    let result = (|| {
        let engine = Engine::new(dd.clone(), default_env, std::env::var_os("QUAYFLOW_OFFLINE").is_some())?;
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?);
        let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("no port")?;
        let conn = EngineConn {
            port,
            token: engine.token.clone(),
            pid: std::process::id(),
            protocol: qf_model::PROTOCOL,
            version: qf_model::ENGINE_VERSION.into(),
            started_at: now_ms(),
        };
        qf_vault::atomic_write(
            &dd.engine_json(),
            serde_json::to_string(&conn).map_err(|e| e.to_string())?.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let mut workers = vec![];
        for _ in 0..crate::http::HTTP_WORKERS {
            let (s, e) = (server.clone(), engine.clone());
            workers.push(std::thread::spawn(move || {
                while !e.shutdown.load(Ordering::SeqCst) {
                    match s.recv_timeout(Duration::from_millis(500)) {
                        // `handle` answers a panicking route with 500; this guard keeps the
                        // worker alive should anything else in the request handling panic.
                        Ok(Some(req)) => {
                            let _ =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::http::handle(&e, req)));
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            }));
        }
        let mut last = std::time::Instant::now() - tick;
        while !engine.shutdown.load(Ordering::SeqCst) {
            if last.elapsed() >= tick {
                let e = engine.clone();
                // A panic in one tick must not take the engine down.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.tick()));
                last = std::time::Instant::now();
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        for w in workers {
            let _ = w.join();
        }
        engine.log("info", "engine.stop", serde_json::json!({}));
        Ok(true)
    })();
    release_instance_lock(&dd, lock);
    result
}
