//! Loopback API v1 (FR-20): per-run token, bounded bodies, validation, versioned handshake,
//! duplicate-request protection. No arbitrary command endpoint; no token-bearing responses.

use crate::accounts::ApiErr;
use crate::core::{CanaryRun, Engine, Shared};
use qf_model::api::*;
use qf_model::time::now_ms;
use qf_model::*;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::Ordering;
use std::time::Duration;

const MAX_BODY: usize = 64 * 1024;

/// HTTP worker threads.
pub const HTTP_WORKERS: usize = 12;
/// Event long-polls that may wait at once; the remaining workers stay free for the
/// per-request routes (/v1/step, /v1/adoption, /v1/recover). Extra pollers get an
/// immediate answer.
pub const MAX_LONG_POLLS: usize = HTTP_WORKERS - 3;

fn q(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode `%XX` and `+` on bytes (never slices the string, so any input is safe).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let (Some(h), Some(l)) = (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                    out.push(h * 16 + l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// One of the [`MAX_LONG_POLLS`] long-poll slots, released on drop (also on a panic).
pub struct LongPollSlot<'a>(&'a Engine);

impl<'a> LongPollSlot<'a> {
    pub fn take(e: &'a Engine) -> Option<Self> {
        e.long_polls
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_LONG_POLLS).then_some(n + 1)
            })
            .ok()
            .map(|_| LongPollSlot(e))
    }
}

impl Drop for LongPollSlot<'_> {
    fn drop(&mut self) {
        self.0.long_polls.fetch_sub(1, Ordering::SeqCst);
    }
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value, ApiErr> {
    serde_json::to_value(v).map_err(|e| ApiErr::new(500, "internal", e.to_string()))
}

fn parse<T: serde::de::DeserializeOwned>(body: &Value) -> Result<T, ApiErr> {
    serde_json::from_value(body.clone()).map_err(|e| ApiErr::invalid(format!("invalid request: {e}")))
}

fn s<'a>(body: &'a Value, k: &str) -> Option<&'a str> {
    body.get(k).and_then(|v| v.as_str())
}

fn strings(body: &Value, k: &str) -> Vec<String> {
    body.get(k)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn env_param(e: &Engine, body: &Value) -> EnvId {
    match s(body, "env").filter(|x| !x.is_empty()) {
        Some(x) => EnvId(x.into()),
        None => match s(body, "sessionId") {
            Some(sid) => e.env_of_session(&e.lock(), sid),
            None => e.default_env.id.clone(),
        },
    }
}

fn refusal(r: crate::transition::Refusal) -> SwitchResponse {
    SwitchResponse {
        ok: false,
        transition: None,
        message: Some(r.message),
        blocked_by: r.blocked_by.map(|b| *b),
    }
}

pub fn route(e: &Shared, method: &str, path: &str, query: &str, body: &Value) -> Result<Value, ApiErr> {
    match (method, path) {
        ("POST", "/v1/hello") => {
            let h: HelloRequest = parse(body)?;
            if let (false, Some(d)) = (h.headless, &h.desktop) {
                qf_adapter::process::set_desktop(d);
            }
            let accepted: Vec<u32> = (PROTOCOL.saturating_sub(1).max(1)..=PROTOCOL).collect();
            let compatible = accepted.contains(&h.protocol);
            let message = (!compatible).then(|| {
                format!(
                    "The Quayflow Mod (protocol {}) and engine {} (protocol {}) do not match. Run `quayflow update` to install a matched pair. Claude works normally meanwhile; switching and recovery are off.",
                    h.protocol, ENGINE_VERSION, PROTOCOL
                )
            });
            let inner = e.lock();
            e.record(
                &inner,
                "hello",
                json!({"protocol": h.protocol, "mod": h.mod_version, "compatible": compatible, "headless": h.headless, "claudeVersion": h.claude_version}),
                false,
            );
            ok(HelloResponse {
                compatible,
                protocol: PROTOCOL,
                accepted_protocols: accepted,
                engine_version: ENGINE_VERSION.into(),
                message,
            })
        }
        ("GET", "/v1/state") => ok(e.state_view(q(query, "sessionId").as_deref(), q(query, "cwd").as_deref())),
        ("GET", "/v1/events") => {
            let since = q(query, "since").and_then(|v| v.parse().ok()).unwrap_or(0);
            let wait = q(query, "wait")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                .min(25);
            // Bounded long-polls: past the cap, answer at once (the Mod then pauses briefly).
            let slot = (wait > 0).then(|| LongPollSlot::take(e)).flatten();
            let wait = if slot.is_some() { wait } else { 0 };
            let r = e.wait_events(since, Duration::from_secs(wait));
            drop(slot);
            ok(r)
        }
        ("POST", "/v1/onboarding") => {
            e.onboarding(
                body.get("consents").unwrap_or(&json!({})),
                body.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
            )?;
            ok(OkResponse::ok("saved"))
        }
        ("POST", "/v1/accounts/register") => {
            let env = s(body, "env").map(|x| EnvId(x.into()));
            let (id, note) = e.register(
                env.as_ref(),
                s(body, "label").unwrap_or(""),
                s(body, "group").unwrap_or(""),
                body.get("acceptDisclosure").and_then(|v| v.as_bool()).unwrap_or(false),
            )?;
            let view = e.state_view(None, None).accounts.into_iter().find(|a| a.id == id);
            let mut v = ok(view)?;
            if let (Some(n), Some(o)) = (note, v.as_object_mut()) {
                o.insert("note".into(), json!(n));
            }
            Ok(v)
        }
        ("POST", "/v1/migrate-poc") => {
            // No group named: the configured default group.
            let group = match s(body, "group").filter(|g| !g.trim().is_empty()) {
                Some(g) => g.to_string(),
                None => e.lock().config.default_group.clone(),
            };
            let lines = e.migrate_poc(
                &group,
                body.get("acceptDisclosure").and_then(|v| v.as_bool()).unwrap_or(false),
            )?;
            ok(OkResponse::ok(lines.join(
                "
",
            )))
        }
        ("POST", "/v1/accounts/signin/start") => ok(e.signin_start(s(body, "reauth"))?),
        ("GET", "/v1/accounts/signin/status") => ok(e.signin_status(&q(query, "id").unwrap_or_default())?),
        ("POST", "/v1/accounts/signin/confirm") => {
            let msg = e.signin_confirm(
                s(body, "id").unwrap_or(""),
                s(body, "label"),
                s(body, "group"),
                body.get("acceptDisclosure").and_then(|v| v.as_bool()).unwrap_or(false),
                body.get("asNew").and_then(|v| v.as_bool()).unwrap_or(false),
            )?;
            ok(OkResponse::ok(msg))
        }
        ("POST", "/v1/accounts/signin/cancel") => {
            e.signin_cancel(s(body, "id").unwrap_or(""))?;
            ok(OkResponse::ok("cancelled; the live login is unchanged"))
        }
        ("POST", "/v1/accounts/remove") => {
            e.remove_account(s(body, "account").unwrap_or(""))?;
            ok(OkResponse::ok(
                "removed from Quayflow (history and provider-side authorization are untouched)",
            ))
        }
        ("POST", "/v1/accounts/label") => {
            e.relabel(s(body, "account").unwrap_or(""), s(body, "label").unwrap_or(""))?;
            ok(OkResponse::ok("label updated"))
        }
        ("POST", "/v1/groups/members") => ok(e.group_members(
            s(body, "group").unwrap_or(""),
            &strings(body, "add"),
            &strings(body, "remove"),
            body.get("acceptDisclosure").and_then(|v| v.as_bool()).unwrap_or(false),
        )?),
        ("POST", p) if p.starts_with("/v1/groups/") => ok(OkResponse::ok(e.group_op(&p["/v1/groups/".len()..], body)?)),
        ("POST", "/v1/projects/assign") => ok(OkResponse::ok(e.project_op(
            "assign",
            s(body, "path").unwrap_or(""),
            s(body, "group"),
        )?)),
        ("POST", "/v1/projects/unassign") => ok(OkResponse::ok(e.project_op(
            "unassign",
            s(body, "path").unwrap_or(""),
            None,
        )?)),
        ("POST", "/v1/switch") => {
            let r: SwitchRequest = parse(body)?;
            let env = env_param(e, body);
            let to = {
                let inner = e.lock();
                e.find_account(&inner, &r.to)?
            };
            Ok(
                match e.start_switch(&env, &to, SwitchHow::Manual, "your choice", r.idle_wait, false) {
                    Ok(t) => ok(SwitchResponse {
                        ok: true,
                        transition: Some(t),
                        message: None,
                        blocked_by: None,
                    })?,
                    Err(rf) => ok(refusal(rf))?,
                },
            )
        }
        ("POST", "/v1/accept") => {
            let env = env_param(e, body);
            let sug = e.lock().envs.get(&env).and_then(|s| s.suggestion.clone());
            let Some(sug) = sug else {
                return Err(ApiErr::refused("no pending suggestion"));
            };
            // Accepting is approval of timing, not a manual choice: no pin.
            let how = SwitchHow::Accepted;
            Ok(match e.start_switch(&env, &sug.to, how, &sug.reason, false, false) {
                Ok(t) => ok(SwitchResponse {
                    ok: true,
                    transition: Some(t),
                    message: None,
                    blocked_by: None,
                })?,
                Err(rf) => ok(refusal(rf))?,
            })
        }
        ("POST", "/v1/transition/cancel") => {
            let msg = e.cancel(s(body, "id").unwrap_or("")).map_err(ApiErr::not_found)?;
            let started = msg.starts_with("Transition already started");
            Ok(json!({"ok": true, "message": msg, "started": started}))
        }
        ("POST", "/v1/unpin") => {
            let env = env_param(e, body);
            e.unpin(&env);
            ok(OkResponse::ok("pin cleared"))
        }
        ("POST", "/v1/step") => ok(e.step(&parse(body)?)),
        ("POST", "/v1/adoption") => ok(OkResponse::ok(e.adoption(&parse(body)?).map_err(ApiErr::invalid)?)),
        ("POST", "/v1/recover") => ok(e.recover(&parse(body)?)),
        ("POST", "/v1/failure") => {
            e.record_failure(&parse(body)?);
            ok(OkResponse::ok("recorded"))
        }
        ("POST", "/v1/unknown") => {
            let env = env_param(e, body);
            match s(body, "action") {
                Some("ignore") => ok(OkResponse::ok(e.ignore_unknown(&env)?)),
                _ => Err(ApiErr::invalid("action: ignore (register uses /v1/accounts/register)")),
            }
        }
        ("GET", "/v1/lanes") => ok(e.lane_views()),
        ("POST", "/v1/lanes/create") => ok(e.lane_create(s(body, "name").unwrap_or(""))?),
        ("POST", "/v1/lanes/check") => ok(e.lane_check(s(body, "name").unwrap_or(""))?),
        ("POST", "/v1/lanes/delete") => ok(OkResponse::ok(e.lane_delete(s(body, "name").unwrap_or(""))?)),
        ("POST", p) if p.starts_with("/v1/handoff/") => ok(e.handoff(
            &p["/v1/handoff/".len()..],
            s(body, "sessionId").unwrap_or(""),
            s(body, "toEnv").unwrap_or(""),
        )?),
        ("POST", "/v1/engine") => match s(body, "action") {
            Some("pause") | Some("resume") => {
                let pause = s(body, "action") == Some("pause");
                let mut inner = e.lock();
                inner.config.automation_paused = pause;
                e.save_config(&inner).map_err(ApiErr::invalid)?;
                if !pause {
                    // Resuming also lifts a hold left by an unresolved switch, but not one over
                    // inconsistent login files: only a switch repairs those.
                    for st in inner.envs.values_mut().filter(|s| s.mixed.is_none()) {
                        st.hold = None;
                    }
                }
                e.record(&inner, "automation", json!({"paused": pause}), false);
                let text = if pause {
                    "Automation paused. Manual switching still works."
                } else {
                    "Automation resumed."
                };
                e.emit(&mut inner, "state", None, text);
                ok(OkResponse::ok(text))
            }
            Some("stop") => {
                let _ = std::fs::write(e.dd.stopped_marker(), now_ms().to_string());
                e.shutdown.store(true, Ordering::SeqCst);
                e.events_cv.notify_all();
                ok(OkResponse::ok("engine stopping; Claude keeps working without Quayflow"))
            }
            _ => Err(ApiErr::invalid("action: pause, resume or stop")),
        },
        ("GET", "/v1/policy") => {
            let inner = e.lock();
            let envs: Vec<Value> = inner
                .envs
                .iter()
                .map(|(id, st)| json!({"env": id, "managed": st.managed}))
                .collect();
            ok(json!({"envs": envs, "dOrg": inner.config.d_org}))
        }
        ("POST", "/v1/policy/dorg") => ok(OkResponse::ok(e.set_dorg(
            body.get("enable").and_then(|v| v.as_bool()).unwrap_or(false),
            s(body, "confirmOrg"),
            s(body, "acknowledge"),
        )?)),
        ("GET", "/v1/compat") => ok(e.lock().compat.values().cloned().collect::<Vec<_>>()),
        ("POST", "/v1/compat/check") => {
            let mut inner = e.lock();
            let versions: Vec<String> = inner.compat.keys().cloned().collect();
            for v in versions {
                let mut rec = crate::core::static_check(&v);
                if let Some(old) = inner.compat.get(&v) {
                    rec.live_pass = old.live_pass;
                    rec.live_detail = old.live_detail.clone();
                }
                e.save_compat(&rec);
                inner.compat.insert(v, rec);
            }
            let ids: Vec<EnvId> = inner.envs.keys().cloned().collect();
            for id in ids {
                e.refresh_env(&mut inner, &id);
            }
            ok(inner.compat.values().cloned().collect::<Vec<_>>())
        }
        ("POST", "/v1/compat/reset") => {
            // After a Claude update or a manual canary pass, forget a recorded live failure.
            let v = s(body, "version").unwrap_or("").to_string();
            let mut inner = e.lock();
            let rec = inner
                .compat
                .get_mut(&v)
                .ok_or_else(|| ApiErr::not_found("no record for that version"))?;
            rec.live_pass = body.get("livePass").and_then(|x| x.as_bool());
            rec.live_detail = s(body, "detail").map(String::from);
            let rec = rec.clone();
            e.save_compat(&rec);
            e.record(
                &inner,
                "compat.live",
                json!({"version": v, "livePass": rec.live_pass}),
                true,
            );
            ok(rec)
        }
        ("POST", "/v1/canary/start") => {
            let mut inner = e.lock();
            if inner.config.consents.live_canary.is_none() {
                return Err(ApiErr::refused(
                    "the live canary needs consent (quayflow consent live-canary on)",
                ));
            }
            inner.canary = Some(CanaryRun {
                since: now_ms(),
                steps: vec![],
            });
            ok(OkResponse::ok("canary recording"))
        }
        ("GET", "/v1/canary") => ok(e
            .lock()
            .canary
            .as_ref()
            .map(|c| json!({"since": c.since, "steps": c.steps}))),
        ("POST", "/v1/canary/stop") => {
            let c = e.lock().canary.take();
            ok(c.map(|c| json!({"since": c.since, "steps": c.steps})))
        }
        ("POST", "/v1/canary/switch") => {
            let env = env_param(e, body);
            let to = {
                let inner = e.lock();
                e.find_account(&inner, s(body, "to").unwrap_or(""))?
            };
            Ok(
                match e.start_switch(&env, &to, SwitchHow::Canary, "version canary", false, true) {
                    Ok(t) => ok(SwitchResponse {
                        ok: t.state.written(),
                        transition: Some(t),
                        message: None,
                        blocked_by: None,
                    })?,
                    Err(rf) => ok(refusal(rf))?,
                },
            )
        }
        ("POST", "/v1/report") => {
            let (path, preview) = e.build_report().map_err(ApiErr::invalid)?;
            ok(ReportResponse { path, preview })
        }
        ("POST", "/v1/report/preview") => ok(e.preview_report().map_err(ApiErr::invalid)?),
        ("POST", "/v1/report/save") => {
            let (path, preview) = e
                .save_reviewed_report(s(body, "id").unwrap_or(""))
                .map_err(ApiErr::refused)?;
            ok(ReportResponse { path, preview })
        }
        ("GET", "/v1/explain") => {
            let env = match q(query, "env").filter(|x| !x.is_empty()) {
                Some(x) => EnvId(x),
                None => match q(query, "sessionId") {
                    Some(sid) => e.env_of_session(&e.lock(), &sid),
                    None => e.default_env.id.clone(),
                },
            };
            ok(e.explain(&env, q(query, "sessionId").as_deref())?)
        }
        ("GET", "/v1/identity") => {
            let env = q(query, "env").filter(|x| !x.is_empty()).map(EnvId);
            ok(e.identity(env.as_ref(), q(query, "account").as_deref())?)
        }
        ("GET", "/v1/handoff/check") => ok(e.handoff_check(
            &q(query, "sessionId").unwrap_or_default(),
            &q(query, "toEnv").unwrap_or_default(),
        )?),
        ("GET", "/v1/operations") => {
            let limit = q(query, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20)
                .clamp(1, 100);
            ok(e.operations(limit))
        }
        ("POST", "/v1/pin") => {
            let env = env_param(e, body);
            ok(e.pin_live(&env)?)
        }
        ("POST", "/v1/tuning") => ok(OkResponse::ok(e.set_tuning(body)?)),
        ("POST", "/v1/poll") => {
            e.observe(true);
            ok(OkResponse::ok("polled"))
        }
        _ => Err(ApiErr::not_found("unknown route")),
    }
}

/// Constant-time equality (the time taken does not depend on where the inputs differ).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

/// Handle one HTTP request (token, body limit, request-id cache).
pub fn handle(e: &Shared, mut req: tiny_http::Request) {
    let method = req.method().as_str().to_string();
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let token_ok = req
        .headers()
        .iter()
        .any(|h| h.field.equiv("x-quayflow-token") && ct_eq(h.value.as_str().as_bytes(), e.token.as_bytes()));
    let respond = |req: tiny_http::Request, status: u16, body: Value| {
        let header = tiny_http::Header::from_bytes(&b"content-type"[..], &b"application/json"[..]).expect("header");
        let _ = req.respond(
            tiny_http::Response::from_string(body.to_string())
                .with_status_code(status)
                .with_header(header),
        );
    };
    if !token_ok {
        return respond(req, 403, json!({"error": "bad token", "code": "bad_token"}));
    }
    let mut raw = Vec::new();
    let read = req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut raw);
    if read.is_err() || raw.len() > MAX_BODY {
        return respond(req, 413, json!({"error": "request too large", "code": "invalid"}));
    }
    let body: Value = if raw.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&raw) {
            Ok(v) => v,
            Err(_) => return respond(req, 400, json!({"error": "body is not JSON", "code": "invalid"})),
        }
    };
    // Duplicate protection: the key is (method, path, requestId); the entry is claimed before
    // the route runs, so a concurrent retry gets 409 instead of running the request twice.
    let request_key = body
        .get("requestId")
        .and_then(|v| v.as_str())
        .map(|rid| format!("{method} {path} {rid}"));
    if let Some(key) = &request_key {
        let mut inner = e.lock();
        match inner.request_cache.iter().find(|(k, _)| k == key) {
            Some((_, Some(v))) => return respond(req, 200, v.clone()),
            Some((_, None)) => {
                drop(inner);
                return respond(
                    req,
                    409,
                    json!({"error": "this request is already being handled", "code": "in_progress"}),
                );
            }
            None => {
                inner.request_cache.push_back((key.clone(), None));
                while inner.request_cache.len() > 256 {
                    // Evict the oldest finished entry; in-flight claims stay.
                    match inner.request_cache.iter().position(|(_, v)| v.is_some()) {
                        Some(i) => {
                            inner.request_cache.remove(i);
                        }
                        None => break,
                    }
                }
            }
        }
    }
    // A panic while handling one request answers 500; the worker thread survives.
    let routed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route(e, &method, &path, &query, &body)))
        .unwrap_or_else(|p| {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            Err(ApiErr::new(500, "internal", format!("internal error: {what}")))
        });
    if let Some(key) = &request_key {
        let mut inner = e.lock();
        let pos = inner.request_cache.iter().position(|(k, _)| k == key);
        match (&routed, pos) {
            // A success is remembered for retries; a failure releases the claim (retry runs).
            (Ok(v), Some(i)) => inner.request_cache[i].1 = Some(v.clone()),
            (Err(_), Some(i)) => {
                inner.request_cache.remove(i);
            }
            _ => {}
        }
    }
    match routed {
        Ok(v) => respond(req, 200, v),
        Err(err) => {
            if err.status >= 500 {
                e.log("error", "api.error", json!({"path": path, "error": err.message}));
            }
            respond(req, err.status, json!({"error": err.message, "code": err.code}))
        }
    }
}
