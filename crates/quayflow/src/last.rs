//! `quayflow last --session <id> [--json]`: what Quayflow did for one Claude session, read
//! from the engine's journal (works with the engine stopped). For unattended `claude -p`
//! jobs: Claude's output and exit code stay Claude's own; a job that wants to know why a
//! run failed, whether it was recovered, or when to run again asks here.
//!
//! The journal keeps account ids only; names come from config.json, here, for local output.

use qf_model::time::to_rfc3339;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Exit code when nothing is recorded for the session.
pub const NOT_FOUND: i32 = 3;

fn window_name(kind: &str) -> String {
    match kind {
        "five_hour" => "5h".into(),
        "seven_day" => "7d".into(),
        k => match k.strip_prefix("seven_day_") {
            Some(model) => format!("7d {model}"),
            None => k.to_string(),
        },
    }
}

fn name(labels: &HashMap<String, String>, id: Option<&str>) -> Value {
    match id {
        Some(i) if !i.is_empty() => Value::String(labels.get(i).cloned().unwrap_or_else(|| format!("#{i}"))),
        _ => Value::Null,
    }
}

fn session_of(e: &Value) -> Option<&str> {
    if let Some(s) = e.get("session").and_then(Value::as_str) {
        return Some(s);
    }
    // Records written before the decision carried its session: the key starts with it.
    e.get("key").and_then(Value::as_str).and_then(|k| k.split(':').next())
}

/// The fullest window of a failure's rate limits (the latest reset on a tie).
fn fullest(shape: &Value) -> Option<(String, f64, Option<String>)> {
    let mut best: Option<(String, f64, Option<String>)> = None;
    for l in shape.get("rateLimits").and_then(Value::as_array).into_iter().flatten() {
        let Some(pct) = l.get("percentUsed").and_then(Value::as_f64) else {
            continue;
        };
        let kind = l.get("kind").and_then(Value::as_str).unwrap_or("window").to_string();
        let resets = l.get("resetsAt").and_then(Value::as_str).map(str::to_string);
        let better = match &best {
            None => true,
            Some((_, p, r)) => pct > *p || (pct == *p && resets > *r),
        };
        if better {
            best = Some((kind, pct, resets));
        }
    }
    best
}

/// The summary for `session` from journal records (oldest first).
pub fn summarize(events: &[Value], labels: &HashMap<String, String>, session: &str) -> Value {
    let mine: Vec<&Value> = events.iter().filter(|e| session_of(e) == Some(session)).collect();
    let ts = |e: &Value| e.get("ts").and_then(Value::as_i64).unwrap_or(0);
    let event = |e: &Value, n: &str| e.get("event").and_then(Value::as_str) == Some(n);
    let decision = mine.iter().rev().find(|e| event(e, "recovery.decision")).copied();
    let at = decision.map(ts);
    let shape = mine
        .iter()
        .rev()
        .find(|e| event(e, "failure.shape") && at.is_none_or(|t| ts(e) <= t))
        .copied();
    if decision.is_none() && shape.is_none() {
        return json!({
            "session": session, "found": false, "outcome": "no_limit",
            "limit": null, "switchedTo": null, "retried": false, "availableAt": null,
            "why": "nothing recorded for this session: no usage limit reached Quayflow (or the Mod was not running)",
        });
    }
    let from = decision.and_then(|d| d.get("from")).and_then(Value::as_str);
    let limit = shape.and_then(fullest).map(|(kind, pct, resets)| {
        json!({"account": name(labels, from), "window": window_name(&kind), "pct": pct, "resetsAt": resets})
    });
    let Some(d) = decision else {
        return json!({
            "session": session, "found": true, "outcome": "not_recovered", "limit": limit,
            "switchedTo": null, "retried": false, "availableAt": limit.as_ref().and_then(|l| l.get("resetsAt").cloned()),
            "why": "the failure was reported, but no recovery was decided (the engine was busy or unreachable); Claude's own result stands",
        });
    };
    let retry = d.get("retry").and_then(Value::as_bool).unwrap_or(false);
    let switched = d.get("switched").and_then(Value::as_bool).unwrap_or(false);
    let to = d.get("to").and_then(Value::as_str);
    let soonest = d.get("availableAt").and_then(Value::as_i64);
    // The journal names accounts by id only; this output is local, so it names them.
    let mut why = d.get("why").and_then(Value::as_str).unwrap_or_default().to_string();
    for (id, label) in labels {
        why = why.replace(&format!("#{id}"), label);
    }
    // A retry that failed too is reported after the decision, without a decision of its own.
    let retry_failed = retry && mine.iter().any(|e| event(e, "failure.shape") && ts(e) > ts(d));
    let limit_reset = limit
        .as_ref()
        .and_then(|l| l.get("resetsAt").cloned())
        .unwrap_or(Value::Null);
    let (outcome, available, why) = if retry && !retry_failed {
        ("recovered", Value::Null, why)
    } else if retry {
        let on = name(labels, to);
        (
            "not_recovered",
            limit_reset,
            format!(
                "retried on {}, and the retry failed too ({why})",
                on.as_str().unwrap_or("?")
            ),
        )
    } else if switched && soonest.is_some() {
        (
            "all_exhausted",
            soonest.map(|t| Value::String(to_rfc3339(t))).unwrap_or(Value::Null),
            why,
        )
    } else {
        ("not_recovered", limit_reset, why)
    };
    json!({
        "session": session,
        "found": true,
        "outcome": outcome,
        "at": at.map(to_rfc3339),
        "limit": limit,
        "switchedTo": if switched || retry { name(labels, to) } else { Value::Null },
        "retried": retry,
        "availableAt": available,
        "why": why,
    })
}

/// The human form of a summary.
pub fn text(v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut out = format!("Session {}: {}", s("session"), s("outcome").replace('_', " "));
    if let Some(l) = v.get("limit").filter(|l| !l.is_null()) {
        let acct = l.get("account").and_then(Value::as_str).unwrap_or("the live account");
        let pct = l.get("pct").and_then(Value::as_f64).unwrap_or(0.0);
        let win = l.get("window").and_then(Value::as_str).unwrap_or("?");
        out.push_str(&format!("\n  limit:        {acct} at {pct:.0}% {win}"));
        if let Some(r) = l.get("resetsAt").and_then(Value::as_str) {
            out.push_str(&format!(" (resets {r})"));
        }
    }
    if let Some(t) = v.get("switchedTo").and_then(Value::as_str) {
        out.push_str(&format!(
            "\n  switched to:  {t}{}",
            if v["retried"] == true {
                " (request retried once)"
            } else {
                ""
            }
        ));
    }
    if let Some(a) = v.get("availableAt").and_then(Value::as_str) {
        out.push_str(&format!("\n  run again at: {a}"));
    }
    out.push_str(&format!("\n  why:          {}", s("why")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> HashMap<String, String> {
        [("aaa", "sdk"), ("bbb", "personal")]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect()
    }
    fn shape(ts: i64, sid: &str) -> Value {
        json!({"ts": ts, "event": "failure.shape", "session": sid, "rateLimits": [
            {"kind": "five_hour", "percentUsed": 1.0, "resetsAt": "2026-11-07T13:30:00Z"},
            {"kind": "seven_day", "percentUsed": 100.0, "resetsAt": "2026-11-08T04:00:00Z"}]})
    }
    fn decision(ts: i64, sid: &str, retry: bool, switched: bool, available: Option<i64>) -> Value {
        json!({"ts": ts, "event": "recovery.decision", "key": format!("{sid}:t:0:main"), "session": sid, "from": "aaa",
               "retry": retry, "switched": switched, "to": "bbb", "why": "#aaa hit a limit; #bbb has room", "availableAt": available})
    }

    #[test]
    fn recovered() {
        let v = summarize(&[shape(1, "S"), decision(2, "S", true, true, None)], &labels(), "S");
        assert_eq!(v["outcome"], "recovered");
        assert_eq!(v["limit"]["account"], "sdk");
        assert_eq!(v["limit"]["window"], "7d");
        assert_eq!(v["limit"]["pct"], 100.0);
        assert_eq!(v["switchedTo"], "personal");
        assert_eq!(v["retried"], true);
        assert!(v["availableAt"].is_null());
        assert_eq!(
            v["why"], "sdk hit a limit; personal has room",
            "ids named in local output"
        );
    }

    #[test]
    fn all_exhausted_gives_the_soonest_time() {
        let v = summarize(
            &[shape(1, "S"), decision(2, "S", false, true, Some(1_794_100_000_000))],
            &labels(),
            "S",
        );
        assert_eq!(v["outcome"], "all_exhausted");
        assert_eq!(v["retried"], false);
        assert_eq!(v["availableAt"], to_rfc3339(1_794_100_000_000));
    }

    #[test]
    fn suggest_mode_is_not_recovered_and_waits_for_the_failed_account() {
        let v = summarize(&[shape(1, "S"), decision(2, "S", false, false, None)], &labels(), "S");
        assert_eq!(v["outcome"], "not_recovered");
        assert!(v["switchedTo"].is_null());
        assert_eq!(v["availableAt"], "2026-11-08T04:00:00Z");
    }

    #[test]
    fn a_failed_retry_is_not_recovered() {
        let v = summarize(
            &[shape(1, "S"), decision(2, "S", true, true, None), shape(3, "S")],
            &labels(),
            "S",
        );
        assert_eq!(v["outcome"], "not_recovered");
        assert!(v["why"].as_str().unwrap().starts_with("retried on personal"));
    }

    #[test]
    fn unknown_session_and_other_sessions_are_not_mixed() {
        let ev = [shape(1, "OTHER"), decision(2, "OTHER", true, true, None)];
        let v = summarize(&ev, &labels(), "S");
        assert_eq!(v["found"], false);
        assert_eq!(v["outcome"], "no_limit");
    }

    #[test]
    fn older_records_without_session_or_from_still_match_by_key() {
        let mut d = decision(2, "S", true, true, None);
        d.as_object_mut().unwrap().remove("session");
        d.as_object_mut().unwrap().remove("from");
        let v = summarize(&[shape(1, "S"), d], &labels(), "S");
        assert_eq!(v["outcome"], "recovered");
        assert!(v["limit"]["account"].is_null());
        assert!(text(&v).contains("the live account at 100% 7d"));
    }
}
