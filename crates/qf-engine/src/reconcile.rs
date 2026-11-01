//! Crash reconciliation (FR-19, AC-17, AC-28): after a restart, unfinished transitions in
//! the journal are resolved against the real environment state. Nothing is replayed.

use crate::core::Engine;
use qf_model::AccountId;
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
    let inner = engine.lock();
    for (id, rec) in open {
        let env_id = rec.get("env").and_then(|v| v.as_str()).unwrap_or("default").to_string();
        let to = rec.get("to").and_then(|v| v.as_str()).map(|s| AccountId(s.into()));
        let from = rec.get("from").and_then(|v| v.as_str()).map(|s| AccountId(s.into()));
        let env = inner
            .envs
            .get(&qf_model::EnvId(env_id.clone()))
            .map(|s| s.env.clone())
            .unwrap_or_else(|| engine.default_env.clone());
        let live = qf_adapter::read_identity(&env).map(|u| AccountId::from_uuid(&u));
        let outcome = if !rec.get("applying").is_some_and(|v| v == true) {
            "not started before the engine stopped; login unchanged"
        } else if live.is_some() && live == to {
            "the write completed before the engine stopped; clients adopt at their next request"
        } else if live.is_some() && live == from {
            "the write did not happen; login unchanged"
        } else {
            "the live login matches neither side; automation stays paused until it is resolved"
        };
        engine.record(
            &inner,
            "transition.reconciled",
            json!({"id": id, "env": env_id, "live": live, "outcome": outcome}),
            true,
        );
    }
}
