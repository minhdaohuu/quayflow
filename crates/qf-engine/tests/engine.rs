//! Engine scenarios against a fake Claude environment (temporary folders, real DPAPI vault,
//! no network). Real-login behavior is covered by the owner-run scripts in tests/e2e.

use qf_adapter::Env;
use qf_engine::{DataDir, Engine, Shared};
use qf_model::api::*;
use qf_model::*;
use serde_json::json;
use std::path::Path;

struct World {
    _dir: tempfile::TempDir,
    env: Env,
    dd: DataDir,
}

const VERSION: &str = "9.9.9";

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

fn session_file(env: &Env, sid: &str, cwd: &str, status: &str) {
    std::fs::create_dir_all(env.sessions_dir()).unwrap();
    let pid = std::process::id();
    std::fs::write(
        env.sessions_dir().join(format!("{pid}.json")),
        json!({"pid": pid, "sessionId": sid, "cwd": cwd, "version": VERSION, "status": status, "kind": "interactive"})
            .to_string(),
    )
    .unwrap();
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::default_at(&dir.path().join("home"));
    let dd = DataDir::at(&dir.path().join("qf"));
    dd.ensure().unwrap();
    // A passing static check for the fake Claude version.
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
    World { _dir: dir, env, dd }
}

const SID: &str = "11111111-2222-4333-8444-555555555555";

/// Two registered accounts: "org" (live) and "personal", both in Default.
fn engine_with_accounts(w: &World) -> Shared {
    login_files(&w.env, "uuid-personal", "P1", "max");
    session_file(&w.env, SID, "C:\\proj", "idle");
    let e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    e.onboarding(
        &json!({"credentialStorage": true, "terms": true, "engineStart": true}),
        true,
    )
    .unwrap();
    e.register(None, "personal", "Default", false).unwrap();
    login_files(&w.env, "uuid-org", "O1", "team");
    e.watch();
    // Mixed identities: the disclosure is required once, then recorded.
    let err = e.register(None, "org", "Default", false).unwrap_err();
    assert_eq!(err.code, "needs_disclosure");
    assert!(err.message.contains("organization") && err.message.contains("personal"));
    e.register(None, "org", "Default", true).unwrap();
    e.tick();
    e
}

fn id(uuid: &str) -> AccountId {
    AccountId::from_uuid(uuid)
}

fn live(env: &Env) -> Option<String> {
    qf_adapter::read_identity(env)
}

fn set_obs(e: &Shared, uuid: &str, pct: f64, resets_in_ms: Option<i64>) {
    let mut inner = e.lock();
    let now = time::now_ms();
    qf_engine::core::merge_observation(
        &mut inner.observations,
        Observation {
            account: id(uuid),
            ts: now,
            windows: vec![Window {
                key: "5h".into(),
                pct,
                resets_at: resets_in_ms.map(|d| now + d),
            }],
            source: "engine".into(),
        },
        false,
    );
}

fn set_mode(e: &Shared, mode: &str) {
    e.group_op(
        "mode",
        &json!({"group": "Default", "mode": mode, "approveAutomatic": true}),
    )
    .unwrap();
}

#[test]
fn register_requires_consent_and_group() {
    let w = world();
    login_files(&w.env, "uuid-a", "A", "max");
    let e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    assert_eq!(e.register(None, "a", "Default", false).unwrap_err().code, "refused");
    e.onboarding(&json!({"credentialStorage": true, "terms": true}), true)
        .unwrap();
    assert_eq!(e.register(None, "a", "Nope", false).unwrap_err().code, "invalid");
    e.register(None, "a", "Default", false).unwrap();
    // Duplicate is not extra capacity.
    let (_, note) = e.register(None, "again", "Default", false).unwrap();
    assert!(note.unwrap().contains("already registered"));
    // Registration never switches and never enables Automatic.
    assert_eq!(live(&w.env).as_deref(), Some("uuid-a"));
    assert_eq!(e.lock().config.groups[0].mode, Mode::Suggest);
    // Vault holds no plaintext.
    let vault_text = std::fs::read_to_string(w.dd.vault().join(format!("{}.dpapi", id("uuid-a")))).unwrap();
    assert!(!vault_text.contains("sk-ant"));
}

#[test]
fn manual_switch_pins_and_tracks_adoption() {
    let w = world();
    let e = engine_with_accounts(&w);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap();
    assert_eq!(t.state, TransitionState::VerifyingAdoption);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
    // Other keys untouched; the outgoing org login was captured, not lost.
    let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&w.env.claude_json).unwrap()).unwrap();
    assert_eq!(cfg["numStartups"], 1);
    let st = e.state_view(Some(SID), None);
    let env = &st.envs[0];
    assert!(env.pin.is_some(), "manual switch pins");
    assert_eq!(env.adoption[0].state, AdoptionState::PendingNextRequest);
    // First request after the write: the Mod is asked to verify, then reports.
    let r = e.step(&StepReport {
        session_id: SID.into(),
        ok: true,
        ..Default::default()
    });
    assert!(r.verify);
    e.adoption(&AdoptionReport {
        session_id: SID.into(),
        account: Some(id("uuid-personal")),
        transition: r.transition,
        ..Default::default()
    })
    .unwrap();
    let st = e.state_view(Some(SID), None);
    assert_eq!(st.envs[0].adoption[0].state, AdoptionState::Adopted);
    assert_eq!(
        st.envs[0].transition.as_ref().unwrap().state,
        TransitionState::Completed
    );
    assert_eq!(st.session.unwrap().verified_account, Some(id("uuid-personal")));
    // Nothing more to verify.
    assert!(
        !e.step(&StepReport {
            session_id: SID.into(),
            ok: true,
            ..Default::default()
        })
        .verify
    );
    // The watcher sees our own write, not an external change.
    e.watch();
    assert!(!e.lock().envs[&EnvId::default_env()].other_switcher);
}

#[test]
fn request_in_flight_at_the_switch_is_not_adoption_evidence() {
    let w = world();
    let e = engine_with_accounts(&w);
    let before = qf_model::time::now_ms() - 5_000;
    e.start_switch(
        &EnvId::default_env(),
        &id("uuid-personal"),
        SwitchHow::Manual,
        "test",
        false,
        true,
    )
    .unwrap();
    // A request that started before the write finishes on the old account: no verification asked.
    let r = e.step(&StepReport {
        session_id: SID.into(),
        ok: true,
        started_at: Some(before),
        ..Default::default()
    });
    assert!(!r.verify, "in-flight request is not checked");
    // Even if its identity arrives (live canary), it does not decide adoption.
    let m = e
        .adoption(&AdoptionReport {
            session_id: SID.into(),
            account: Some(id("uuid-org")),
            transition: None,
            started_at: Some(before),
        })
        .unwrap();
    assert!(m.contains("in flight"), "{m}");
    let st = e.state_view(Some(SID), None);
    assert_eq!(st.envs[0].adoption[0].state, AdoptionState::PendingNextRequest);
    assert!(st.envs[0].in_place.allowed, "in-place switching stays on");
    // The next request, started after the write, is the evidence.
    let after = qf_model::time::now_ms();
    let r = e.step(&StepReport {
        session_id: SID.into(),
        ok: true,
        started_at: Some(after),
        ..Default::default()
    });
    assert!(r.verify);
    e.adoption(&AdoptionReport {
        session_id: SID.into(),
        account: Some(id("uuid-personal")),
        transition: r.transition,
        started_at: Some(after),
    })
    .unwrap();
    assert_eq!(
        e.state_view(Some(SID), None).envs[0].adoption[0].state,
        AdoptionState::Adopted
    );
}

#[test]
fn adoption_mismatch_disables_in_place_for_the_version() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.start_switch(
        &EnvId::default_env(),
        &id("uuid-personal"),
        SwitchHow::Manual,
        "test",
        false,
        true,
    )
    .unwrap();
    let r = e.step(&StepReport {
        session_id: SID.into(),
        ok: true,
        ..Default::default()
    });
    assert_eq!(
        e.adoption(&AdoptionReport {
            session_id: SID.into(),
            account: Some(id("uuid-org")),
            transition: r.transition,
            ..Default::default()
        })
        .unwrap(),
        "mismatch"
    );
    e.tick();
    let st = e.state_view(None, None);
    assert!(!st.envs[0].in_place.allowed);
    assert!(st
        .compat
        .iter()
        .any(|c| c.version == VERSION && c.live_pass == Some(false)));
    let refused = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-org"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap_err();
    assert!(refused.message.contains("disabled"));
    // The live canary can still run: it is how the version is checked again.
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-org"),
            SwitchHow::Canary,
            "version canary",
            false,
            true,
        )
        .unwrap();
    assert!(t.state.written());
}

#[test]
fn unpin_during_an_in_flight_switch_is_not_lost() {
    let w = world();
    let e = engine_with_accounts(&w);
    {
        let mut inner = e.lock();
        inner.config.tuning.lock_wait_s = 5;
    }
    // Claude holds its refresh lock, so the switch waits in flight.
    std::fs::create_dir(w.env.refresh_lock()).unwrap();
    let e2 = e.clone();
    let switch = std::thread::spawn(move || {
        e2.start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !e.lock().envs[&EnvId::default_env()].blocking_transition() {
        assert!(std::time::Instant::now() < deadline, "switch never started");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    e.unpin(&EnvId::default_env());
    std::fs::remove_dir(w.env.refresh_lock()).unwrap();
    let t = switch.join().unwrap();
    assert_eq!(t.state, TransitionState::VerifyingAdoption);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
    assert!(
        e.state_view(None, None).envs[0].pin.is_none(),
        "the unpin given mid-switch holds"
    );
}

#[test]
fn held_lock_leaves_login_unchanged() {
    let w = world();
    let e = engine_with_accounts(&w);
    {
        let mut inner = e.lock();
        inner.config.tuning.lock_wait_s = 1;
    }
    std::fs::create_dir(w.env.refresh_lock()).unwrap();
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap();
    assert_eq!(t.state, TransitionState::Failed);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    std::fs::remove_dir(w.env.refresh_lock()).unwrap();
}

#[test]
fn recovery_automatic_switches_once_and_retries_once() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_mode(&e, "automatic");
    set_obs(&e, "uuid-personal", 10.0, None);
    let req = |idx: u64| RecoverRequest {
        session_id: SID.into(),
        turn_id: Some("t".into()),
        index: Some(idx),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(100.0),
            resets_at: None,
        }],
        ..Default::default()
    };
    let r = e.recover(&req(1));
    assert!(r.retry && r.switched, "{}", r.why);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
    // Same request again: one recovery per request.
    let again = e.recover(&req(1));
    assert!(!again.retry);
    // Another concurrent failure (another subagent) shares the switch.
    let mut other = req(2);
    other.agent_id = Some("sub".into());
    let c = e.recover(&other);
    assert!(c.retry && !c.switched, "{}", c.why);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
}

#[test]
fn recovery_refuses_unconfirmed_limits_and_suggest_mode_only_records() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_obs(&e, "uuid-personal", 10.0, None);
    // Generic error: not a usage limit.
    let r = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(1),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(40.0),
            resets_at: None,
        }],
        ..Default::default()
    });
    assert!(!r.retry && !r.switched && r.why.contains("not a confirmed"));
    // Suggest mode: confirmed limit records a suggestion, no switch (headless never prompts).
    let r = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(2),
        simulated: true,
        ..Default::default()
    });
    assert!(!r.switched && r.why.contains("Suggest"));
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    assert!(e.state_view(None, None).envs[0].suggestion.is_some());
}

#[test]
fn all_exhausted_switches_to_soonest_without_retry() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_mode(&e, "automatic");
    set_obs(&e, "uuid-org", 100.0, Some(5 * 3_600_000));
    set_obs(&e, "uuid-personal", 100.0, Some(3_600_000));
    let r = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(1),
        simulated: true,
        ..Default::default()
    });
    assert!(r.switched && !r.retry, "{}", r.why);
    assert!(r.why.contains("soonest"));
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
}

#[test]
fn model_not_found_after_switch_switches_back() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_mode(&e, "automatic");
    e.start_switch(
        &EnvId::default_env(),
        &id("uuid-personal"),
        SwitchHow::Automatic,
        "test",
        false,
        true,
    )
    .unwrap();
    let r = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(1),
        model: Some("claude-opus-5-5".into()),
        error_kind: Some("model_not_found".into()),
        ..Default::default()
    });
    assert!(r.switched && r.retry, "{}", r.why);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    let inner = e.lock();
    assert!(inner
        .config
        .account(&id("uuid-personal"))
        .unwrap()
        .unavailable_models
        .contains(&"claude-opus-5-5".to_string()));
}

#[test]
fn unknown_login_pauses_and_manual_switch_backs_it_up() {
    let w = world();
    let e = engine_with_accounts(&w);
    login_files(&w.env, "uuid-stranger", "S1", "pro");
    e.watch();
    e.tick();
    let st = e.state_view(None, None);
    assert!(st.envs[0].notices.iter().any(|n| n.kind == "unknown_login"));
    assert!(st.envs[0].automation_paused);
    set_mode(&e, "automatic");
    let r = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(1),
        simulated: true,
        ..Default::default()
    });
    assert!(!r.switched && r.why.contains("paused"));
    // Ignore keeps automation paused (B1).
    e.ignore_unknown(&EnvId::default_env()).unwrap();
    assert!(e.state_view(None, None).envs[0].automation_paused);
    // Manual switch works and keeps a protected backup of the unknown login.
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-org"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap();
    assert!(t.state.written());
    let backups: Vec<_> = std::fs::read_dir(w.dd.backups()).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    assert!(!std::fs::read_to_string(backups[0].path()).unwrap().contains("sk-ant"));
    assert!(!e.state_view(None, None).envs[0].automation_paused);
}

#[test]
fn repeated_external_changes_flag_another_switcher() {
    let w = world();
    let e = engine_with_accounts(&w);
    let bump = || std::thread::sleep(std::time::Duration::from_millis(30));
    login_files(&w.env, "uuid-personal", "P2", "max");
    bump();
    e.watch();
    login_files(&w.env, "uuid-org", "O2", "team");
    bump();
    e.watch();
    let st = e.state_view(None, None);
    assert!(st.envs[0].notices.iter().any(|n| n.kind == "other_switcher"));
}

#[test]
fn native_refresh_is_captured_into_the_vault() {
    let w = world();
    let e = engine_with_accounts(&w);
    login_files(&w.env, "uuid-org", "O-REFRESHED", "team");
    std::thread::sleep(std::time::Duration::from_millis(30));
    e.watch();
    let v = qf_vault::Vault::new(w.dd.vault());
    assert_eq!(
        v.load(id("uuid-org").as_str()).unwrap().refresh_token(),
        Some("sk-ant-ort-O-REFRESHED")
    );
    assert!(!e.lock().envs[&EnvId::default_env()].other_switcher);
}

#[test]
fn group_conflict_names_the_blocking_session() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.group_op("create", &json!({"name": "Work"})).unwrap();
    e.group_members("Work", &["org".into()], &[], false).unwrap();
    e.project_op("assign", "C:\\proj", Some("Work")).unwrap();
    e.tick();
    let r = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap_err();
    assert_eq!(r.code, "conflict");
    assert!(
        r.message.contains("Work") && r.message.contains("lane"),
        "{}",
        r.message
    );
    assert_eq!(r.blocked_by.unwrap().session_id, SID);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"), "refused before any write");
}

#[test]
fn proactive_suggest_then_accept_without_pin() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_obs(&e, "uuid-org", 90.0, None);
    set_obs(&e, "uuid-personal", 20.0, None);
    e.decide(&EnvId::default_env());
    let st = e.state_view(None, None);
    let sug = st.envs[0].suggestion.clone().expect("suggestion");
    assert_eq!(sug.to, id("uuid-personal"));
    assert_eq!(
        live(&w.env).as_deref(),
        Some("uuid-org"),
        "suggest never switches by itself"
    );
    let v = qf_engine::http::route(&e, "POST", "/v1/accept", "", &json!({})).unwrap();
    assert_eq!(v["ok"], true);
    for _ in 0..50 {
        if live(&w.env).as_deref() == Some("uuid-personal") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(e.state_view(None, None).envs[0].pin.is_none(), "accepting does not pin");
}

#[test]
fn crash_reconciliation_never_replays() {
    let w = world();
    let e = engine_with_accounts(&w);
    let org = id("uuid-org");
    let personal = id("uuid-personal");
    drop(e);
    // A transition that was requested and applying when the engine died; the write happened.
    let j = qf_engine::journal::Journal::new(w.dd.journal());
    j.append(
        "transition.requested",
        json!({"id": "dead1", "env": "default", "from": org, "to": personal, "how": "automatic"}),
        &[],
        true,
    )
    .unwrap();
    j.append(
        "transition.applying",
        json!({"id": "dead1", "env": "default"}),
        &[],
        true,
    )
    .unwrap();
    // One that never got to apply.
    j.append(
        "transition.requested",
        json!({"id": "dead2", "env": "default", "from": org, "to": personal, "how": "automatic"}),
        &[],
        true,
    )
    .unwrap();
    let before = live(&w.env);
    let _e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    assert_eq!(live(&w.env), before, "nothing replayed");
    let rec: Vec<_> = j
        .read_all()
        .into_iter()
        .filter(|r| r["event"] == "transition.reconciled")
        .collect();
    assert_eq!(rec.len(), 2);
    assert!(rec
        .iter()
        .any(|r| r["id"] == "dead1" && r["outcome"].as_str().unwrap().contains("did not happen")));
    assert!(rec
        .iter()
        .any(|r| r["id"] == "dead2" && r["outcome"].as_str().unwrap().contains("not started")));
    // Already reconciled: a second restart finds nothing open.
    let _e2 = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    assert_eq!(
        j.read_all()
            .iter()
            .filter(|r| r["event"] == "transition.reconciled")
            .count(),
        2
    );
}

#[test]
fn handshake_rejects_incompatible_protocol() {
    let w = world();
    let e = engine_with_accounts(&w);
    let ok = qf_engine::http::route(&e, "POST", "/v1/hello", "", &json!({"protocol": 1})).unwrap();
    assert_eq!(ok["compatible"], true);
    let bad = qf_engine::http::route(&e, "POST", "/v1/hello", "", &json!({"protocol": 7})).unwrap();
    assert_eq!(bad["compatible"], false);
    assert!(bad["message"].as_str().unwrap().contains("quayflow update"));
}

#[test]
fn journal_and_report_hold_no_secrets() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.start_switch(
        &EnvId::default_env(),
        &id("uuid-personal"),
        SwitchHow::Manual,
        "test",
        false,
        true,
    )
    .unwrap();
    let (path, _) = e.build_report().unwrap();
    // The label "personal" must not appear as a value anywhere, except in the fixed-vocabulary
    // `kind` field whose value happens to be the same word.
    fn label_leaks(v: &serde_json::Value, key: &str) -> bool {
        match v {
            serde_json::Value::String(s) => {
                key != "kind" && s.split(|c: char| !c.is_alphanumeric()).any(|w| w == "personal")
            }
            serde_json::Value::Array(a) => a.iter().any(|x| label_leaks(x, key)),
            serde_json::Value::Object(o) => o.iter().any(|(k, x)| label_leaks(x, k)),
            _ => false,
        }
    }
    for p in [w.dd.journal(), Path::new(&path).to_path_buf()] {
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("sk-ant"), "secret leaked");
        let docs: Vec<serde_json::Value> = if p.extension().is_some_and(|x| x == "jsonl") {
            t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
        } else {
            vec![serde_json::from_str(&t).unwrap()]
        };
        assert!(
            !docs.iter().any(|d| label_leaks(d, "")),
            "label leaked into a record: {}",
            p.display()
        );
    }
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        report["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "personal"),
        "the account kind is kept, not replaced by an id"
    );
    let logs: String = std::fs::read_dir(w.dd.logs())
        .unwrap()
        .flatten()
        .map(|f| std::fs::read_to_string(f.path()).unwrap())
        .collect();
    assert!(!logs.contains("sk-ant"));
}

#[test]
fn lanes_switch_independently() {
    let w = world();
    let e = engine_with_accounts(&w);
    let lane = e.lane_create("work").unwrap();
    assert!(lane.links_ok);
    // Lane check passes only when the governing version passes; seed the installed one.
    e.lane_check("work").unwrap();
    let lane_id = EnvId("work".into());
    // One login cannot be live in two environments.
    let r = e.start_switch(&lane_id, &id("uuid-org"), SwitchHow::Manual, "test", false, true);
    if let Err(r) = &r {
        assert!(
            r.message.contains("live in environment") || r.message.contains("disabled"),
            "{}",
            r.message
        );
    }
    let lane_env = qf_adapter::lanes::lane_env(&w.dd.lanes(), "work");
    assert!(qf_adapter::read_identity(&lane_env).is_none());
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    e.lane_delete("work").unwrap();
    assert!(!lane_env.config_dir.exists());
}

/// Wait (bounded) until `f` holds.
fn wait_until(what: &str, secs: u64, f: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while !f() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Run `f` on a thread; the caller waits on the receiver with a timeout, so a deadlock fails
/// the test instead of hanging it.
fn spawn<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

const BOUND: std::time::Duration = std::time::Duration::from_secs(30);

#[test]
fn lane_delete_during_an_in_flight_switch_does_not_deadlock() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.lane_create("work").unwrap();
    e.lock().config.tuning.lock_wait_s = 5;
    // Claude holds its refresh lock: the switch waits in Applying, holding the credential lock.
    std::fs::create_dir(w.env.refresh_lock()).unwrap();
    let e2 = e.clone();
    let switch = spawn(move || {
        e2.start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .map(|t| t.state)
    });
    wait_until("switch never reached Applying", 5, || {
        e.lock().envs[&EnvId::default_env()]
            .transition
            .as_ref()
            .is_some_and(|t| t.state == TransitionState::Applying)
    });
    let e3 = e.clone();
    let delete = spawn(move || e3.lane_delete("work").map_err(|x| x.message));
    std::thread::sleep(std::time::Duration::from_millis(300));
    // The engine lock stays free while the lane delete waits for the credential lock (the old
    // order held it here, and the switch then deadlocked taking it under the credential lock).
    wait_until("lane delete holds the engine lock", 5, || e.inner.try_lock().is_ok());
    std::fs::remove_dir(w.env.refresh_lock()).unwrap();
    let state = switch.recv_timeout(BOUND).expect("switch deadlocked").unwrap();
    assert_eq!(state, TransitionState::VerifyingAdoption);
    delete.recv_timeout(BOUND).expect("lane delete deadlocked").unwrap();
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
    assert!(!e.lock().envs.contains_key(&EnvId("work".into())));
}

#[test]
fn lane_delete_concurrent_with_the_watcher_finishes() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.lane_create("work").unwrap();
    let (e2, env) = (e.clone(), w.env.clone());
    let watcher = spawn(move || {
        for i in 0..20 {
            login_files(&env, "uuid-org", &format!("O-W{i}"), "team");
            e2.watch();
        }
    });
    let e3 = e.clone();
    let delete = spawn(move || e3.lane_delete("work").map_err(|x| x.message));
    watcher.recv_timeout(BOUND).expect("watcher deadlocked");
    delete.recv_timeout(BOUND).expect("lane delete deadlocked").unwrap();
}

fn poc_login(dir: &Path, uuid: &str, token: &str, sub: &str) -> String {
    let env = Env::default_at(&dir.join(format!("src-{uuid}")));
    login_files(&env, uuid, token, sub);
    serde_json::to_string(&qf_adapter::read_login(&env).unwrap().unwrap()).unwrap()
}

#[test]
fn poc_migration_keeps_plaintext_until_the_import_is_saved() {
    let w = world();
    login_files(&w.env, "uuid-personal", "P1", "max");
    let e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    e.onboarding(&json!({"credentialStorage": true, "terms": true}), true)
        .unwrap();
    e.register(None, "personal", "Default", false).unwrap();
    let poc = w._dir.path().join("poc").join("vault");
    std::fs::create_dir_all(poc.join("org")).unwrap();
    let plain = poc.join("org").join("login.json");
    std::fs::write(&plain, poc_login(w._dir.path(), "uuid-org", "O1", "team")).unwrap();
    let vault_file = w.dd.vault().join(format!("{}.dpapi", id("uuid-org")));
    // Mixing identities needs the disclosure: nothing is stored and the PoC login stays.
    let err = e.migrate_poc_from(&poc, "Default", false).unwrap_err();
    assert_eq!(err.code, "needs_disclosure");
    assert!(plain.exists(), "plaintext PoC login kept on refusal");
    assert!(!vault_file.exists(), "nothing sealed before the disclosure");
    assert!(e.lock().config.account(&id("uuid-org")).is_none());
    // An unknown group: refused, nothing touched.
    assert_eq!(e.migrate_poc_from(&poc, "Nope", true).unwrap_err().code, "not_found");
    assert!(plain.exists());
    // Accepted: sealed, saved, then the plaintext goes.
    let out = e.migrate_poc_from(&poc, "Default", true).unwrap();
    assert!(out.iter().any(|l| l.contains("imported")), "{out:?}");
    assert!(vault_file.exists());
    assert!(!plain.exists(), "plaintext removed after the import was saved");
    let saved = std::fs::read_to_string(w.dd.config()).unwrap();
    assert!(saved.contains(id("uuid-org").as_str()));
    assert!(e.lock().config.groups[0].accounts.contains(&id("uuid-org")));
}

#[test]
fn concurrent_failure_waits_for_an_in_flight_recovery_switch_and_shares_it() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_mode(&e, "automatic");
    set_obs(&e, "uuid-personal", 10.0, None);
    e.lock().config.tuning.lock_wait_s = 5;
    let req = |idx: u64, agent: &str| RecoverRequest {
        session_id: SID.into(),
        turn_id: Some("t".into()),
        index: Some(idx),
        agent_id: Some(agent.into()),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(100.0),
            resets_at: None,
        }],
        ..Default::default()
    };
    std::fs::create_dir(w.env.refresh_lock()).unwrap();
    let (e1, r1) = (e.clone(), req(1, "main"));
    let first = spawn(move || e1.recover(&r1));
    wait_until("recovery switch never started", 5, || {
        e.lock().envs[&EnvId::default_env()].blocking_transition()
    });
    let (e2, r2) = (e.clone(), req(2, "sub"));
    let second = spawn(move || e2.recover(&r2));
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::remove_dir(w.env.refresh_lock()).unwrap();
    let a = first.recv_timeout(BOUND).unwrap();
    let b = second.recv_timeout(BOUND).unwrap();
    assert!(a.retry && a.switched, "{}", a.why);
    assert!(b.retry && !b.switched, "{}", b.why);
    assert_eq!(b.to, Some(id("uuid-personal")));
    assert_eq!(b.transition, a.transition, "one switch shared");
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
}

#[test]
fn concurrent_failure_without_limit_evidence_is_not_retried() {
    let w = world();
    let e = engine_with_accounts(&w);
    set_mode(&e, "automatic");
    set_obs(&e, "uuid-personal", 10.0, None);
    let first = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(1),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(100.0),
            resets_at: None,
        }],
        ..Default::default()
    });
    assert!(first.retry && first.switched, "{}", first.why);
    // Another failure right after, with no usage-limit evidence (and none known for the account
    // it ran on): Claude's own result stands.
    let other = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(2),
        agent_id: Some("sub".into()),
        ..Default::default()
    });
    assert!(!other.retry && !other.switched, "{}", other.why);
    assert!(other.why.contains("not a confirmed"), "{}", other.why);
    // A session view below the hard limit is no evidence either.
    let low = e.recover(&RecoverRequest {
        session_id: SID.into(),
        index: Some(3),
        agent_id: Some("sub2".into()),
        rate_limits: vec![SessionRateLimit {
            kind: Some("five_hour".into()),
            percent_used: Some(40.0),
            resets_at: None,
        }],
        ..Default::default()
    });
    assert!(!low.retry, "{}", low.why);
    assert_eq!(live(&w.env).as_deref(), Some("uuid-personal"));
}

const DEAD_SID: &str = "00000000-dead-4000-8000-000000000000";

#[test]
fn plan_check_uses_the_models_of_running_sessions_only() {
    let w = world();
    let e = engine_with_accounts(&w);
    {
        let mut inner = e.lock();
        inner
            .config
            .account_mut(&id("uuid-personal"))
            .unwrap()
            .unavailable_models
            .push("claude-opus-5-5".into());
        // A session that ended (or ran in another environment) reported that model.
        inner.session_model.insert(DEAD_SID.into(), "claude-opus-5-5".into());
    }
    e.tick();
    assert!(
        !e.lock().session_model.contains_key(DEAD_SID),
        "ended sessions are pruned"
    );
    e.lock().session_model.insert(DEAD_SID.into(), "claude-opus-5-5".into());
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap();
    assert!(t.state.written(), "a session not running here does not decide");
    // Back, then the running session reports a model the plan cannot serve: refused.
    e.start_switch(
        &EnvId::default_env(),
        &id("uuid-org"),
        SwitchHow::Manual,
        "test",
        false,
        true,
    )
    .unwrap();
    e.step(&StepReport {
        session_id: SID.into(),
        ok: true,
        model: Some("claude-opus-5-5".into()),
        ..Default::default()
    });
    let r = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap_err();
    assert!(r.message.contains("claude-opus-5-5"), "{}", r.message);
}

#[test]
fn vault_error_clears_when_the_copy_loads_again() {
    let w = world();
    let e = engine_with_accounts(&w);
    let personal = id("uuid-personal");
    e.lock().config.account_mut(&personal).unwrap().health = Health::VaultError;
    e.tick();
    assert_eq!(e.lock().config.account(&personal).unwrap().health, Health::Ok);
    // Capturing a refresh reads the stored copy: that clears it as well.
    let org = id("uuid-org");
    e.lock().config.account_mut(&org).unwrap().health = Health::VaultError;
    login_files(&w.env, "uuid-org", "O-REFRESHED", "team");
    std::thread::sleep(std::time::Duration::from_millis(30));
    e.watch();
    assert_eq!(e.lock().config.account(&org).unwrap().health, Health::Ok);
    // A copy that really cannot be read keeps the mark.
    std::fs::write(w.dd.vault().join(format!("{personal}.dpapi")), "garbage").unwrap();
    e.lock().config.account_mut(&personal).unwrap().health = Health::VaultError;
    e.tick();
    assert_eq!(e.lock().config.account(&personal).unwrap().health, Health::VaultError);
}

#[test]
fn project_assign_to_a_missing_group_keeps_the_assignment() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.group_op("create", &json!({"name": "Work"})).unwrap();
    e.project_op("assign", "C:\\proj", Some("Work")).unwrap();
    let err = e.project_op("assign", "C:\\proj", Some("Nope")).unwrap_err();
    assert_eq!(err.code, "not_found");
    let inner = e.lock();
    assert!(inner
        .config
        .projects
        .iter()
        .any(|p| p.path == "C:\\proj" && p.group == "Work"));
}

#[test]
fn malformed_input_does_not_panic() {
    let w = world();
    let e = engine_with_accounts(&w);
    // A percent escape followed by a multi-byte character.
    qf_engine::http::route(
        &e,
        "GET",
        "/v1/state",
        "sessionId=%a\u{e9}&cwd=%\u{e9}\u{e9}",
        &json!({}),
    )
    .unwrap();
    assert_eq!(qf_engine::http::percent_decode("%41%zz+"), "A%zz ");
    assert_eq!(qf_engine::http::percent_decode("%C3%A9"), "\u{e9}");
    let accents = "\u{e9}".repeat(9);
    assert_eq!(qf_engine::transition::short(&accents), "\u{e9}".repeat(8));
    assert_eq!(qf_engine::transition::short("abc"), "abc");
}

#[test]
fn long_polls_are_capped() {
    let w = world();
    let e = engine_with_accounts(&w);
    let seq = e.seq();
    let query = format!("since={seq}&wait=25");
    let slots: Vec<_> = (0..qf_engine::http::MAX_LONG_POLLS)
        .map(|_| qf_engine::http::LongPollSlot::take(&e).expect("slot"))
        .collect();
    assert!(qf_engine::http::LongPollSlot::take(&e).is_none());
    let t = std::time::Instant::now();
    let v = qf_engine::http::route(&e, "GET", "/v1/events", &query, &json!({})).unwrap();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "past the cap: answered at once"
    );
    assert!(v["events"].as_array().unwrap().is_empty());
    drop(slots);
    assert_eq!(e.long_polls.load(std::sync::atomic::Ordering::SeqCst), 0);
    // With a free slot, a poll waits for the next event.
    let (e2, q2) = (e.clone(), query.clone());
    let poll = spawn(move || qf_engine::http::route(&e2, "GET", "/v1/events", &q2, &json!({})).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(200));
    e.unpin(&EnvId::default_env());
    let v = poll.recv_timeout(BOUND).unwrap();
    assert!(!v["events"].as_array().unwrap().is_empty());
    assert_eq!(e.long_polls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// Write only the credential file (as a half-written switch leaves it).
fn credentials_only(env: &Env, token: &str, sub: &str) {
    let creds = json!({"claudeAiOauth": {"accessToken": format!("sk-ant-oat-{token}"), "refreshToken": format!("sk-ant-ort-{token}"), "expiresAt": 9_999_999_999_999i64, "subscriptionType": sub}});
    std::fs::write(env.credentials(), creds.to_string()).unwrap();
}

fn vault_refresh(e: &Shared, uuid: &str) -> String {
    e.vault
        .load(id(uuid).as_str())
        .unwrap()
        .refresh_token()
        .unwrap()
        .to_string()
}

fn paused(e: &Shared) -> Option<String> {
    let inner = e.lock();
    inner.envs[&EnvId::default_env()].pause_reason(&inner.config)
}

#[test]
fn mixed_login_files_are_held_and_never_captured_under_the_record() {
    let w = world();
    let e = engine_with_accounts(&w);
    assert_eq!(vault_refresh(&e, "uuid-org"), "sk-ant-ort-O1");
    // Credentials are personal's while .claude.json still names org (half-written switch).
    credentials_only(&w.env, "P1", "max");
    e.watch();
    assert_eq!(vault_refresh(&e, "uuid-org"), "sk-ant-ort-O1", "org's copy untouched");
    assert!(paused(&e).unwrap().contains("inconsistent"));
    {
        let inner = e.lock();
        let st = &inner.envs[&EnvId::default_env()];
        assert_eq!(st.mixed, Some(id("uuid-personal")));
        assert!(st.live.is_none());
    }
    // Registering from inconsistent files is refused.
    assert_eq!(e.register(None, "x", "Default", false).unwrap_err().code, "refused");
    // A refresh of the mixed credentials is not stored under org either.
    credentials_only(&w.env, "P2", "max");
    e.watch();
    assert_eq!(vault_refresh(&e, "uuid-org"), "sk-ant-ort-O1");
    // A switch writes a consistent login and lifts the hold; nothing is stored under org.
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-org"),
            SwitchHow::Manual,
            "repair",
            false,
            true,
        )
        .unwrap();
    assert!(t.state.written(), "{t:?}");
    assert_eq!(vault_refresh(&e, "uuid-org"), "sk-ant-ort-O1");
    assert!(paused(&e).is_none_or(|r| !r.contains("inconsistent")));
    assert!(e.lock().envs[&EnvId::default_env()].mixed.is_none());
    let backups = std::fs::read_dir(w.dd.backups()).unwrap().count();
    assert!(backups > 0, "the unattributable refreshed credentials were backed up");
}

#[test]
fn reconcile_detects_a_half_written_switch_and_pauses() {
    let w = world();
    let e = engine_with_accounts(&w);
    let (org, personal) = (id("uuid-org"), id("uuid-personal"));
    drop(e);
    let j = qf_engine::journal::Journal::new(w.dd.journal());
    for (ev, data) in [
        (
            "transition.requested",
            json!({"id": "half", "env": "default", "from": org, "to": personal, "how": "manual"}),
        ),
        ("transition.applying", json!({"id": "half", "env": "default"})),
    ] {
        j.append(ev, data, &[], true).unwrap();
    }
    credentials_only(&w.env, "P1", "max");
    let e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    let rec = j
        .read_all()
        .into_iter()
        .find(|r| r["event"] == "transition.reconciled")
        .unwrap();
    assert!(rec["outcome"].as_str().unwrap().contains("half-written"), "{rec}");
    assert!(paused(&e).unwrap().contains("inconsistent"));
    assert_eq!(e.lock().envs[&EnvId::default_env()].mixed, Some(personal));
    assert_eq!(vault_refresh(&e, "uuid-org"), "sk-ant-ort-O1");
}

#[test]
fn reconcile_neither_side_really_pauses_until_resumed() {
    let w = world();
    let e = engine_with_accounts(&w);
    let personal = id("uuid-personal");
    drop(e);
    let j = qf_engine::journal::Journal::new(w.dd.journal());
    for (ev, data) in [
        (
            "transition.requested",
            json!({"id": "n", "env": "default", "from": "aaaaaaaaaaaa", "to": personal, "how": "automatic"}),
        ),
        ("transition.applying", json!({"id": "n", "env": "default"})),
    ] {
        j.append(ev, data, &[], true).unwrap();
    }
    let e = Engine::new(w.dd.clone(), w.env.clone(), true).unwrap();
    assert!(paused(&e).unwrap().contains("interrupted switch"));
    set_mode(&e, "automatic");
    e.tick();
    assert!(e.lock().envs[&EnvId::default_env()].decision.contains("paused"));
    qf_engine::http::route(&e, "POST", "/v1/engine", "", &json!({"action": "resume"})).unwrap();
    assert!(paused(&e).is_none_or(|r| !r.contains("interrupted switch")));
}

#[test]
fn failed_capture_of_the_outgoing_login_stops_the_switch() {
    let w = world();
    let e = engine_with_accounts(&w);
    // Claude refreshed org's token; its protected copy cannot be read or replaced.
    login_files(&w.env, "uuid-org", "O2", "team");
    let path = w.dd.vault().join(format!("{}.dpapi", id("uuid-org")));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let t = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap();
    assert_eq!(t.state, TransitionState::Failed, "{t:?}");
    assert_eq!(live(&w.env).as_deref(), Some("uuid-org"));
    let creds = std::fs::read_to_string(w.env.credentials()).unwrap();
    assert!(
        creds.contains("sk-ant-ort-O2"),
        "the only fresh token is still in place"
    );
}

fn in_flight_elsewhere(e: &Shared, w: &World, to: &AccountId) {
    let lane = EnvId("lane1".into());
    let mut st = qf_engine::core::EnvState::new(Env::at(lane.clone(), w.dd.root.join("lane1")));
    st.transition = Some(Transition {
        id: "t1".into(),
        env: lane.clone(),
        from: None,
        to: to.clone(),
        how: SwitchHow::Manual,
        reason: "test".into(),
        state: TransitionState::Applying,
        requested_at: 0,
        written_at: None,
        ended_at: None,
        locked_ms: None,
        detail: None,
        failure: None,
    });
    e.lock().envs.insert(lane, st);
}

#[test]
fn a_login_switching_into_another_environment_is_refused_and_not_removable() {
    let w = world();
    let e = engine_with_accounts(&w);
    in_flight_elsewhere(&e, &w, &id("uuid-personal"));
    let r = e
        .start_switch(
            &EnvId::default_env(),
            &id("uuid-personal"),
            SwitchHow::Manual,
            "test",
            false,
            true,
        )
        .unwrap_err();
    assert!(r.message.contains("two environments"), "{}", r.message);
    let err = e.remove_account("personal").unwrap_err();
    assert!(err.message.contains("in progress"), "{}", err.message);
    assert!(e.lock().config.account(&id("uuid-personal")).is_some());
}

#[test]
fn remove_account_keeps_memory_and_disk_in_step() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.remove_account("personal").unwrap();
    assert!(e.lock().config.account(&id("uuid-personal")).is_none());
    let disk = qf_engine::core::load_config(&w.dd).unwrap();
    assert!(disk.account(&id("uuid-personal")).is_none());
    assert!(!e.vault.exists(id("uuid-personal").as_str()));
}

#[test]
fn mode_without_a_group_uses_the_configured_default_group() {
    let w = world();
    let e = engine_with_accounts(&w);
    e.group_op("create", &json!({"group": "Work"})).unwrap();
    e.group_op("default", &json!({"group": "Work"})).unwrap();
    e.group_op("mode", &json!({"mode": "manual"})).unwrap();
    let inner = e.lock();
    assert_eq!(inner.config.group("Work").unwrap().mode, Mode::Manual);
    assert_eq!(inner.config.group("Default").unwrap().mode, Mode::Suggest);
}

#[test]
fn engine_instance_lock_is_exclusive_and_survives_a_crash() {
    let w = world();
    let first = qf_engine::take_instance_lock(&w.dd).unwrap().expect("first engine");
    assert!(
        qf_engine::take_instance_lock(&w.dd).unwrap().is_none(),
        "second engine refused"
    );
    // A crashed engine leaves the file behind with a PID; its handle is gone.
    drop(first);
    std::fs::write(w.dd.engine_lock(), "4000000000").unwrap();
    assert!(qf_engine::take_instance_lock(&w.dd).unwrap().is_some());
    // An empty lock file (an engine that died before writing its PID) is not a holder.
    std::fs::write(w.dd.engine_lock(), "").unwrap();
    assert!(qf_engine::take_instance_lock(&w.dd).unwrap().is_some());
}

#[test]
fn unsafe_claude_versions_never_name_a_file() {
    use qf_engine::core::safe_version_name as ok;
    assert!(ok("2.1.104") && ok("1.0.0-beta+3"));
    for bad in ["", "..", "../x", "..\\x", "1/2", "1\\2", "CON", ".1", "1..2", "1:2"] {
        assert!(!ok(bad), "{bad}");
    }
}
