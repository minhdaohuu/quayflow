//! Quayflow engine: one per Windows user (FR-19), loopback API (FR-20), automation loop.

pub mod accounts;
pub mod core;
pub mod http;
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

/// Single-instance lock: a file created with CREATE_NEW holding our PID.
/// A lock left by a dead process is replaced. Returns false when another engine runs.
pub fn take_instance_lock(dd: &DataDir) -> Result<bool, String> {
    for _ in 0..3 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dd.engine_lock())
        {
            Ok(mut f) => {
                f.write_all(std::process::id().to_string().as_bytes())
                    .map_err(|e| e.to_string())?;
                return Ok(true);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let holder: u32 = std::fs::read_to_string(dd.engine_lock())
                    .ok()
                    .and_then(|t| t.trim().parse().ok())
                    .unwrap_or(0);
                if holder != 0 && holder != std::process::id() && qf_adapter::process::is_alive(holder) {
                    return Ok(false);
                }
                let _ = std::fs::remove_file(dd.engine_lock());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

pub fn release_instance_lock(dd: &DataDir) {
    if std::fs::read_to_string(dd.engine_lock())
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        == Some(std::process::id())
    {
        let _ = std::fs::remove_file(dd.engine_lock());
    }
    if EngineConn::read(dd).is_some_and(|c| c.pid == std::process::id()) {
        let _ = std::fs::remove_file(dd.engine_json());
    }
}

/// Run the engine until stopped. Returns Ok(false) when another engine already runs.
pub fn serve(dd: DataDir, default_env: qf_adapter::Env, tick: Duration) -> Result<bool, String> {
    dd.ensure().map_err(|e| e.to_string())?;
    if !take_instance_lock(&dd)? {
        return Ok(false);
    }
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
        for _ in 0..8 {
            let (s, e) = (server.clone(), engine.clone());
            workers.push(std::thread::spawn(move || {
                while !e.shutdown.load(Ordering::SeqCst) {
                    match s.recv_timeout(Duration::from_millis(500)) {
                        Ok(Some(req)) => crate::http::handle(&e, req),
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
    release_instance_lock(&dd);
    result
}
