//! Recovery right after an engine start, when the other accounts' usage readings are missing
//! or stale: the engine polls them on demand (bounded, outside the engine lock) and decides
//! with fresh readings. A separate test binary: it sets `QUAYFLOW_SIM_API` for its own process,
//! pointing usage calls at a loopback fake server that answers by the token it is sent.

use qf_adapter::Env;
use qf_engine::{DataDir, Engine, Shared};
use qf_model::api::*;
use qf_model::*;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const VERSION: &str = "9.9.9";
const SID: &str = "11111111-2222-4333-8444-555555555555";

/// Usage requests the fake server has answered, by access token.
fn hits() -> &'static Mutex<HashMap<String, usize>> {
    static H: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hits_for(token: &str) -> usize {
    hits()
        .lock()
        .unwrap()
        .get(&format!("sk-ant-oat-{token}"))
        .copied()
        .unwrap_or(0)
}

/// Starts the fake usage API once for this test binary. A token containing `SLOW` is answered
/// after 15 s, `LIMIT` with 429, `FAIL` with 500, `FULL` with 100% used (resets in 2099),
/// anything else with 10% used.
fn fake_api() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::env::set_var("QUAYFLOW_SIM_API", format!("http://127.0.0.1:{port}"));
        let open = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let open = open.clone();
                std::thread::spawn(move || {
                    open.fetch_add(1, Ordering::SeqCst);
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut token = String::new();
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        let l = line.trim_end().to_string();
                        line.clear();
                        if l.is_empty() {
                            break;
                        }
                        if let Some(v) = l.strip_prefix("Authorization: Bearer ").or_else(|| l.strip_prefix("authorization: Bearer ")) {
                            token = v.to_string();
                        }
                    }
                    *hits().lock().unwrap().entry(token.clone()).or_default() += 1;
                    let mut stream = stream;
                    let (status, body) = if token.contains("LIMIT") {
                        ("429 Too Many Requests", "{}".to_string())
                    } else if token.contains("FAIL") {
                        ("500 Internal Server Error", "{}".to_string())
                    } else if token.contains("FULL") {
                        ("200 OK", json!({"five_hour": {"utilization": 100.0, "resets_at": "2099-01-01T00:00:00Z"}}).to_string())
                    } else {
                        if token.contains("SLOW") {
                            std::thread::sleep(Duration::from_secs(15));
                        }
                        ("200 OK", json!({"five_hour": {"utilization": 10.0, "resets_at": "2099-01-01T00:00:00Z"}}).to_string())
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    open.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
    });
}

struct World {
    _dir: tempfile::TempDir,
    env: Env,
    dd: DataDir,
}

fn login_files(env: &Env, uuid: &str, token: &str, sub: &str) {
    std::fs::create_dir_all(&env.config_dir).unwrap();
    let creds = json!({"claudeAiOauth": {"accessToken": format!("sk-ant-oat-{token}"), "refreshToken": format!("sk-ant-ort-{token}"), "expiresAt": 9_999_999_999_999i64, "subscriptionType": sub}});
    std::fs::write(env.credentials(), creds.to_string()).unwrap();
    let mut cfg: serde_json::Value = std::fs::read_to_string(&env.claude_json)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(json!({"numStartups": 1}));
    cfg["oauthAccount"] = json!({"accountUuid": uuid, "organizationUuid": format!("org-{uuid}")});
    std::fs::write(&env.claude_json, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
}

fn session_file(env: &Env) {
    std::fs::create_dir_all(env.sessions_dir()).unwrap();
    let pid = std::process::id();
    std::fs::write(
        env.sessions_dir().join(format!("{pid}.json")),
        json!({"pid": pid, "sessionId": SID, "cwd": "C:\\proj", "version": VERSION, "status": "idle", "kind": "interactive"})
            .to_string(),
    )
    .unwrap();
}

/// Two accounts registered by an offline engine ("org" live, "personal" idle with
/// `personal_token`), Automatic mode; then the engine that serves the test is started fresh
/// over the same data folder, online, with no usage readings and no tick yet.
fn restarted(personal_token: &str) -> (World, Shared) {
    restarted_with(personal_token, None, "ORG")
}

/// As `restarted`, with a third idle account "sdk" using `sdk_token` when given, and the
/// live account "org" using `org_token`.
fn restarted_with(personal_token: &str, sdk_token: Option<&str>, org_token: &str) -> (World, Shared) {
    fake_api();
    let dir = tempfile::tempdir().unwrap();
    let env = Env::default_at(&dir.path().join("home"));
    let dd = DataDir::at(&dir.path().join("qf"));
    dd.ensure().unwrap();
    let rec = CompatRecord {
        version: VERSION.into(),
        checked_at: 0,
        static_pass: true,
        static_detail: vec![],
        live_pass: None,
        live_detail: None,
    };
    std::fs::write(
        dd.compat_dir().join(format!("{VERSION}.json")),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    login_files(&env, "uuid-personal", personal_token, "max");
    session_file(&env);
    {
        let e = Engine::new(dd.clone(), env.clone(), true).unwrap();
        e.onboarding(
            &json!({"credentialStorage": true, "terms": true, "engineStart": true}),
            true,
        )
        .unwrap();
        e.register(None, "personal", "Default", false).unwrap();
        if let Some(t) = sdk_token {
            login_files(&env, "uuid-sdk", t, "team");
            e.watch();
            e.register(None, "sdk", "Default", true).unwrap();
        }
        login_files(&env, "uuid-org", org_token, "team");
        e.watch();
        e.register(None, "org", "Default", true).unwrap();
        e.group_op(
            "mode",
            &json!({"group": "Default", "mode": "automatic", "approveAutomatic": true}),
        )
        .unwrap();
    }
    let e = Engine::new(dd.clone(), env.clone(), false).unwrap();
    e.watch();
    (World { _dir: dir, env, dd }, e)
}

fn limit_hit(idx: u64) -> RecoverRequest {
    RecoverRequest {
        session_id: SID.into(),
        turn_id: Some("t".into()),
        index: Some(idx),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(100.0),
            resets_at: None,
        }],
        ..Default::default()
    }
}

fn journal_kinds(w: &World) -> Vec<String> {
    std::fs::read_to_string(w.dd.journal())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("event").and_then(|k| k.as_str()).map(String::from))
        .collect()
}

#[test]
fn a_limit_before_the_first_tick_polls_the_unknown_account_then_switches_and_retries() {
    let (w, e) = restarted("OK1");
    assert!(!e.first_tick_done.load(Ordering::SeqCst), "no tick has run");
    let r = e.recover(&limit_hit(1));
    assert!(r.retry && r.switched, "{}", r.why);
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-personal"));
    assert_eq!(hits_for("OK1"), 1, "the idle account was polled once, on demand");
    assert!(
        journal_kinds(&w).iter().any(|k| k == "recovery.poll"),
        "the poll is journaled"
    );
}

#[test]
fn a_rate_limited_usage_api_declines_with_no_reading_to_count() {
    let (w, e) = restarted("LIMIT1");
    let r = e.recover(&limit_hit(1));
    assert!(
        !r.retry && !r.switched && r.to.is_none(),
        "no fresh reading, no room: {}",
        r.why
    );
    assert!(r.why.contains("no approved account has known capacity"), "{}", r.why);
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-org"));
    assert_eq!(hits_for("LIMIT1"), 1);
}

#[test]
fn a_slow_usage_api_is_bounded_and_does_not_hold_the_engine_lock() {
    let (_w, e) = restarted("SLOW1");
    let e2 = e.clone();
    let t0 = Instant::now();
    let rx = {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(e2.recover(&limit_hit(1)));
        });
        rx
    };
    // While the poll waits on the network, the engine still answers.
    let until = Instant::now() + Duration::from_secs(5);
    while hits_for("SLOW1") == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(hits_for("SLOW1"), 1, "the poll reached the network");
    let probe = Instant::now();
    let _ = e.state_view(None, None);
    assert!(probe.elapsed() < Duration::from_secs(2), "state served while polling");
    let r = rx.recv_timeout(Duration::from_secs(30)).expect("recovery answered");
    // The usage call times out at 10 s (the agent's own limit); nothing waits for 15 s.
    assert!(t0.elapsed() < Duration::from_secs(14), "bounded: {:?}", t0.elapsed());
    assert!(
        t0.elapsed() >= Duration::from_secs(9),
        "the usage call's own timeout cut the wait"
    );
    assert!(!r.retry, "{}", r.why);
}

#[test]
fn concurrent_failures_poll_once_and_share_one_switch() {
    let (w, e) = restarted("OK2");
    let (a, b) = (e.clone(), e.clone());
    let ta = std::thread::spawn(move || a.recover(&limit_hit(1)));
    let tb = std::thread::spawn(move || {
        let mut r = limit_hit(2);
        r.agent_id = Some("sub".into());
        b.recover(&r)
    });
    let (ra, rb) = (ta.join().unwrap(), tb.join().unwrap());
    assert!(ra.retry && rb.retry, "{} / {}", ra.why, rb.why);
    assert_eq!(
        [ra.switched, rb.switched].iter().filter(|s| **s).count(),
        1,
        "one switch"
    );
    assert_eq!(hits_for("OK2"), 1, "polled once");
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-personal"));
}

#[test]
fn ready_turns_true_after_the_first_tick() {
    let (_w, e) = restarted("OK3");
    let q = |wait: &str| qf_engine::http::route(&e, "GET", "/v1/ready", &format!("wait={wait}"), &json!({})).unwrap();
    assert_eq!(q("0")["ready"], json!(false));
    let t0 = Instant::now();
    assert_eq!(q("1")["ready"], json!(false), "waits, then answers");
    assert!(t0.elapsed() >= Duration::from_millis(900));
    e.tick();
    assert_eq!(q("5")["ready"], json!(true));
}

fn id_of(e: &Shared, label: &str) -> AccountId {
    e.lock()
        .config
        .accounts
        .iter()
        .find(|a| a.label == label)
        .expect("account")
        .id
        .clone()
}

#[test]
fn an_account_at_its_limit_does_not_win_over_one_whose_room_was_not_read() {
    // "sdk" is read fresh at 100% (resets in 2099), "personal" has no reading: without a poll
    // the soonest-available target would be sdk (switch, no retry). Polling finds room.
    let (w, e) = restarted_with("OK4", Some("FULL4"), "ORG");
    let sdk = id_of(&e, "sdk");
    e.observe_accounts(Some(&[sdk]), true, None);
    assert_eq!(hits_for("FULL4"), 1);
    let r = e.recover(&limit_hit(1));
    assert!(r.retry && r.switched, "{}", r.why);
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-personal"));
    assert_eq!(hits_for("OK4"), 1, "the unread account was polled");
}

#[test]
fn a_failed_poll_is_not_repeated_by_the_next_recovery() {
    let (_w, e) = restarted("FAIL1");
    let r1 = e.recover(&limit_hit(1));
    assert!(!r1.retry, "{}", r1.why);
    let r2 = e.recover(&limit_hit(2));
    assert!(!r2.retry, "{}", r2.why);
    assert_eq!(
        hits_for("FAIL1"),
        1,
        "a failing endpoint costs one recovery, not every one"
    );
}

#[test]
fn a_poll_already_running_is_waited_for_only_until_the_deadline() {
    let (_w, e) = restarted("OK5");
    let held = e.observe_lock.lock().unwrap();
    let e2 = e.clone();
    let t0 = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(e2.recover(&limit_hit(1)));
    });
    let r = rx.recv_timeout(Duration::from_secs(14));
    drop(held);
    let r = r.expect("recovery answered while another poll held the lock");
    assert!(
        t0.elapsed() >= Duration::from_secs(9),
        "waited for the deadline: {:?}",
        t0.elapsed()
    );
    assert!(!r.retry, "{}", r.why);
    assert_eq!(hits_for("OK5"), 0, "nothing polled after the deadline");
}

/// A failed request whose session saw no usage percentages (a fresh `claude -p`).
fn limit_hit_without_percentages(idx: u64) -> RecoverRequest {
    RecoverRequest {
        rate_limits: vec![],
        ..limit_hit(idx)
    }
}

#[test]
fn an_unknown_live_usage_is_read_to_confirm_the_limit_then_the_others() {
    let (w, e) = restarted_with("OK6", None, "FULLORG6");
    let r = e.recover(&limit_hit_without_percentages(1));
    assert!(r.retry && r.switched, "{}", r.why);
    assert_eq!(hits_for("FULLORG6"), 1, "the live account was read once");
    assert_eq!(hits_for("OK6"), 1, "then the other account");
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-personal"));
}

#[test]
fn a_live_account_below_its_limit_is_not_a_confirmed_limit() {
    let (w, e) = restarted_with("OK7", None, "ORG7");
    let r = e.recover(&limit_hit_without_percentages(1));
    assert!(!r.retry && !r.switched, "{}", r.why);
    assert!(r.why.contains("not a confirmed usage limit"), "{}", r.why);
    assert_eq!(hits_for("ORG7"), 1);
    assert_eq!(hits_for("OK7"), 0, "no limit, so no other account is read");
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-org"));
}

#[test]
fn a_concurrent_failure_without_percentages_reads_the_account_switched_from_and_shares() {
    // A arrives with the session's percentages and switches org to personal; B (a subagent's
    // request, no percentages) failed on org too: org's usage is read to confirm, and B
    // shares the switch instead of being declined for an unconfirmed limit.
    let (w, e) = restarted_with("OK8", None, "FULLORG8");
    let a = e.recover(&limit_hit(1));
    assert!(a.retry && a.switched, "{}", a.why);
    let mut b = limit_hit_without_percentages(2);
    b.agent_id = Some("sub".into());
    let b = e.recover(&b);
    assert!(b.retry && !b.switched, "{}", b.why);
    assert_eq!(hits_for("FULLORG8"), 1, "the account switched from was read once");
    assert_eq!(hits_for("OK8"), 1, "the new live account was not read again");
    assert_eq!(qf_adapter::read_identity(&w.env).as_deref(), Some("uuid-personal"));
}
