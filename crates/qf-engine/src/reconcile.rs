//! Crash reconciliation (FR-19, AC-17, AC-28): after a restart, unfinished transitions in
//! the journal are resolved against the real environment state. Nothing is replayed.

use crate::core::Engine;
use qf_model::{AccountId, EnvId};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn reconcile(engine: &Engine) {
    let records = engine.journal.read_all();
    // Latest state per transition id.
    let mut open: BTreeMap<String, Value> = BTreeMap::new();
    for r in &records {
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
        let key = if inner.envs.contains_key(&EnvId(env_id.clone())) {
            EnvId(env_id.clone())
        } else {
            engine.default_env.id.clone()
        };
        let env = inner
            .envs
            .get(&key)
            .map(|s| s.env.clone())
            .unwrap_or_else(|| engine.default_env.clone());
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
        let (found, outcome) = if !applying {
            (Found::Settled, "not started before the engine stopped; login unchanged")
        } else if live.is_some() && live == to {
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
        engine.record(
            &inner,
            "transition.reconciled",
            json!({"id": id, "env": env_id, "live": live, "outcome": outcome}),
            true,
        );
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
