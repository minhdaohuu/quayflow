//! Crash reconciliation (FR-19, AC-17, AC-28): after a restart, unfinished transitions in
//! the journal are resolved against the real environment state. Nothing is replayed.

use crate::core::Engine;
use qf_model::{AccountId, EnvId};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Resolve unfinished transitions, then restore the proactive cooldown; the journal is read once.
pub fn reconcile(engine: &Engine) {
    let records = match engine.journal.try_read_all() {
        Ok(r) => r,
        Err(e) => {
            // Not "no switches": say so. (A switch writes the journal first, so while it
            // cannot be read, it most likely cannot be written either and no switch starts.)
            engine.log.log(
                "warn",
                "journal.unreadable",
                json!({"error": e.kind().to_string(), "effect": "interrupted switches not checked; cooldown not restored"}),
                &[],
            );
            vec![]
        }
    };
    resolve_open(engine, &records);
    restore_cooldown(engine, &records);
}

fn resolve_open(engine: &Engine, records: &[Value]) {
    // Latest state per transition id.
    let mut open: BTreeMap<String, Value> = BTreeMap::new();
    for r in records {
        let Some(id) = r.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        match r.get("event").and_then(|v| v.as_str()).unwrap_or("") {
            "transition.requested" => {
                open.insert(id.into(), r.clone());
            }
            "transition.applying" => {
                if let Some(o) = open.get_mut(id) {
                    o["applying"] = json!(true);
                }
            }
            "transition.written"
            | "transition.ended"
            | "transition.completed"
            | "transition.reconciled"
            | "transition.uncertain" => {
                open.remove(id);
            }
            _ => {}
        }
    }
    if open.is_empty() {
        return;
    }
    let mut inner = engine.lock();
    for (id, rec) in open {
        let env_id = rec.get("env").and_then(|v| v.as_str()).unwrap_or("default").to_string();
        let to = rec.get("to").and_then(|v| v.as_str()).map(|s| AccountId(s.into()));
        let from = rec.get("from").and_then(|v| v.as_str()).map(|s| AccountId(s.into()));
        let key = EnvId(env_id.clone());
        let Some(env) = inner.envs.get(&key).map(|s| s.env.clone()) else {
            // A deleted lane, or an id an older journal scrubbed: never check another
            // environment's login in its place.
            engine.record(
                &inner,
                "transition.reconciled",
                json!({"id": id, "env": env_id, "outcome": "its environment no longer exists; nothing checked"}),
                true,
            );
            continue;
        };
        // Both files: the account record names the identity; the credentials may still be the
        // destination's when the account record was not written.
        let now = qf_adapter::read_login(&env).ok().flatten();
        let live = now.as_ref().and_then(|l| l.account_hash()).map(AccountId);
        let creds_are_to = match (&now, &to) {
            (Some(l), Some(t)) => engine
                .vault
                .load(t.as_str())
                .ok()
                .and_then(|s| s.refresh_token().map(String::from))
                .is_some_and(|rt| l.refresh_token() == Some(rt.as_str())),
            _ => false,
        };
        let applying = rec.get("applying").is_some_and(|v| v == true);
        enum Found {
            Settled,
            Mixed(AccountId),
            Neither,
        }
        let mut written = false;
        let (found, outcome) = if !applying {
            (Found::Settled, "not started before the engine stopped; login unchanged")
        } else if live.is_some() && live == to {
            written = true;
            (
                Found::Settled,
                "the write completed before the engine stopped; clients adopt at their next request",
            )
        } else if let (true, Some(t)) = (creds_are_to, &to) {
            (
                Found::Mixed(t.clone()),
                "half-written: the destination's credentials under another account record; automation is paused until a switch repairs it",
            )
        } else if live == from {
            (Found::Settled, "the write did not happen; login unchanged")
        } else {
            (
                Found::Neither,
                "the live login matches neither side; automation stays paused until a switch completes or you resume it",
            )
        };
        // The switch's own time and kind go with the record, so a later restart restores the
        // cooldown from when it switched, not from when this start found out.
        let switched_at = rec
            .get("ts")
            .and_then(|v| v.as_i64())
            .unwrap_or(i64::MAX)
            .min(qf_model::time::now_ms());
        let how = rec.get("how").cloned().unwrap_or(Value::Null);
        engine.record(
            &inner,
            "transition.reconciled",
            json!({"id": id, "env": env_id, "live": live, "outcome": outcome, "written": written,
                   "switchedAt": switched_at, "how": how}),
            true,
        );
        if written && how != json!("canary") {
            // The switch happened: it holds the cooldown from its own time (the record just
            // written is not among the records `restore_cooldown` reads in this start).
            let at = switched_at;
            if let Some(st) = inner.envs.get_mut(&key) {
                st.last_proactive = Some(st.last_proactive.map_or(at, |t| t.max(at)));
            }
        }
        if let Found::Mixed(owner) = &found {
            engine.hold_mixed(&mut inner, &key, owner, live.as_ref());
        } else if let (Found::Neither, Some(st)) = (&found, inner.envs.get_mut(&key)) {
            st.hold = Some(
                "an interrupted switch left a login that matches neither side; check it, then switch or resume (quayflow resume)".into(),
            );
            let text = format!("After a restart: {outcome}.");
            engine.emit(&mut inner, "notice", Some(&key), &text);
        }
    }
}

/// The proactive cooldown survives a restart: each environment's last switch is the latest
/// `transition.written` record in the journal (a canary switch does not count, as in
/// `after_write`), or a `transition.reconciled` one that found the write completed. Without
/// it, a restarted engine could switch again at once. A time ahead of the clock counts as now.
fn restore_cooldown(engine: &Engine, records: &[Value]) {
    let now = qf_model::time::now_ms();
    let mut inner = engine.lock();
    for r in records {
        let switched = match r.get("event").and_then(|v| v.as_str()) {
            Some("transition.written") => r.get("how").and_then(|v| v.as_str()) != Some("canary"),
            Some("transition.reconciled") => {
                r.get("written") == Some(&json!(true)) && r.get("how").and_then(|v| v.as_str()) != Some("canary")
            }
            _ => false,
        };
        if !switched {
            continue;
        }
        // A recovered switch counts from when it switched (`switchedAt`), not from the restart
        // that found it; an older record without it uses its own time.
        let at = r.get("switchedAt").or_else(|| r.get("ts")).and_then(|v| v.as_i64());
        let (Some(env), Some(ts)) = (r.get("env").and_then(|v| v.as_str()), at) else {
            continue;
        };
        if let Some(st) = inner.envs.get_mut(&EnvId(env.into())) {
            let ts = ts.min(now);
            st.last_proactive = Some(st.last_proactive.map_or(ts, |t| t.max(ts)));
        }
    }
}
