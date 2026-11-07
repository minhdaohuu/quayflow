//! Hard-limit recovery (FR-15, FR-16) and plan-compatibility fallback (FR-09).
//!
//! One switch and at most one retry per failed request; concurrent failures share one
//! switch; a new incident needs genuine task progress; Suggest mode only records.

use crate::core::{random_hex, Engine, Inner};
use qf_model::api::{FailureShape, RecoverRequest, RecoverResponse};
use qf_model::time::now_ms;
use qf_model::*;
use qf_policy::RecoveryTarget;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn no(why: impl Into<String>) -> RecoverResponse {
    RecoverResponse {
        retry: false,
        switched: false,
        to: None,
        to_label: None,
        why: why.into(),
        transition: None,
        available_at: None,
    }
}

impl Engine {
    pub fn record_failure(&self, f: &FailureShape) {
        let inner = self.lock();
        self.record(
            &inner,
            "failure.shape",
            json!({
                "session": f.session_id, "model": f.model, "stopReason": f.stop_reason, "answerEmpty": f.answer_empty,
                "toolUses": f.tool_uses, "hasUsage": f.has_usage, "errorKind": f.error_kind, "fields": f.fields,
                "rateLimits": f.rate_limits, "simulated": f.simulated
            }),
            true,
        );
    }

    pub fn recover(self: &Arc<Self>, r: &RecoverRequest) -> RecoverResponse {
        let key = format!(
            "{}:{}:{}:{}",
            r.session_id,
            r.turn_id.as_deref().unwrap_or("-"),
            r.index.map(|i| i.to_string()).unwrap_or_default(),
            r.agent_id.as_deref().unwrap_or("main")
        );
        // The account the failed request used: the environment's live login before any switch.
        let from = {
            let inner = self.lock();
            let env = self.env_of_session(&inner, &r.session_id);
            inner.envs.get(&env).and_then(|s| s.live.clone())
        };
        let resp = self.recover_inner(r, &key);
        let inner = self.lock();
        self.record(
            &inner,
            "recovery.decision",
            json!({"key": key, "session": r.session_id, "from": from, "retry": resp.retry, "switched": resp.switched, "to": resp.to, "why": resp.why, "availableAt": resp.available_at, "simulated": r.simulated, "errorKind": r.error_kind}),
            true,
        );
        resp
    }

    fn recover_inner(self: &Arc<Self>, r: &RecoverRequest, key: &str) -> RecoverResponse {
        let now = now_ms();
        let mut inner = self.lock();
        if inner.incidents.contains_key(key) {
            return no("this request was already recovered once");
        }
        inner.incidents.insert(key.to_string(), now);
        inner.incidents.retain(|_, t| now - *t < 6 * 3_600_000);
        if let Some(m) = &r.model {
            inner.session_model.insert(r.session_id.clone(), m.clone());
        }
        let env = self.env_of_session(&inner, &r.session_id);
        let Some(st) = inner.envs.get(&env) else {
            return no("unknown environment");
        };
        let live = st.live.clone();
        let last = st.last_switch.clone().or_else(|| st.transition.clone());
        let tuning = inner.config.tuning.clone();

        // Plan compatibility / dead sign-in right after a switch: mark and switch back,
        // as the incident's one recovery (FR-09, FR-04).
        if let (Some(kind), Some(t)) = (r.error_kind.as_deref(), last.as_ref()) {
            let recent = t.written_at.is_some_and(|w| now - w < 10 * 60_000) && live.as_ref() == Some(&t.to);
            if recent && (kind == "model_not_found" || kind == "auth") {
                let Some(from) = t.from.clone() else {
                    return no("no previous account to return to");
                };
                let to_label = inner.config.label(&t.to);
                if let Some(a) = inner.config.account_mut(&t.to) {
                    if kind == "model_not_found" {
                        if let Some(m) = &r.model {
                            if !a.unavailable_models.iter().any(|x| x == m) {
                                a.unavailable_models.push(m.clone());
                            }
                        }
                    } else {
                        a.health = Health::RefreshRejected;
                    }
                }
                let _ = self.save_config(&inner);
                let reason = if kind == "model_not_found" {
                    format!(
                        "{to_label} cannot serve {}; switching back",
                        r.model.as_deref().unwrap_or("this model")
                    )
                } else {
                    format!("{to_label}'s sign-in was not accepted; set aside until you sign in again; switching back")
                };
                let text = if kind == "auth" {
                    format!(
                        "{to_label} needs a new sign-in (/quay accounts → Reauthenticate). It is set aside meanwhile."
                    )
                } else {
                    reason.clone()
                };
                self.emit(&mut inner, "notice", Some(&env), &text);
                drop(inner);
                return match self.start_switch(&env, &from, SwitchHow::ModelFallback, &reason, false, true) {
                    Ok(t) if t.state.written() => RecoverResponse {
                        retry: true,
                        switched: true,
                        to: Some(from.clone()),
                        to_label: Some(self.lock().config.label(&from)),
                        why: reason,
                        transition: Some(t.id),
                        available_at: None,
                    },
                    Ok(t) => no(format!("switch back not made: {}", t.detail.unwrap_or_default())),
                    Err(e) => no(format!("switch back not made: {}", e.message)),
                };
            }
        }

        let st = inner.envs.get(&env).expect("env");
        if let Some(why) = st.pause_reason(&inner.config) {
            return no(format!("automation paused: {why}"));
        }
        // Concurrent failure while a shareable switch is still in flight: wait (bounded, without
        // the engine lock) for it to be written, then share it instead of starting another.
        let in_flight = st
            .transition
            .as_ref()
            .filter(|t| st.blocking_transition() && shareable(t.how))
            .map(|t| t.id.clone());
        let (live, last) = match in_flight {
            None => (live, last),
            Some(tid) => {
                drop(inner);
                let ended = self.wait_transition(&env, &tid, concurrent_wait(&tuning));
                inner = self.lock();
                let Some(t) = ended else {
                    return no("a switch for a concurrent failure is still in progress; Claude's own result stands");
                };
                if !t.state.written() {
                    return no(format!(
                        "the switch for a concurrent failure was not made: {}",
                        t.detail.unwrap_or_default()
                    ));
                }
                let live = inner.envs.get(&env).and_then(|s| s.live.clone());
                (live, Some(t))
            }
        };
        let pcts: Vec<(String, f64)> = r
            .rate_limits
            .iter()
            .filter_map(|l| Some((l.kind.clone().unwrap_or_else(|| "window".into()), l.percent_used?)))
            .collect();
        // Concurrent failure: retry on the account already switched to, while it is live.
        if let Some(t) = last.as_ref() {
            let shared = shareable(t.how)
                && t.written_at
                    .is_some_and(|w| now_ms() - w < tuning.concurrent_window_s as i64 * 1000)
                && live.as_ref() == Some(&t.to);
            if shared {
                return self.share(&inner, r, &env, t, &pcts);
            }
        }
        let governing = Engine::governing(&inner, &env);
        let mode = qf_policy::effective_mode(&governing);
        // A new incident requires genuine task progress since the previous recovery.
        if inner.progressed.get(&r.session_id) == Some(&false) {
            return no("no task progress since the previous recovery; Claude's own result stands");
        }
        let model = r.model.clone();
        let Some(ctx) = Engine::ctx(&inner, &env, model.as_deref()) else {
            return no("unknown environment");
        };
        let evidence = qf_policy::confirm_limit(&ctx, &pcts, r.simulated);
        if !evidence.confirmed {
            return no(format!(
                "not a confirmed usage limit ({}); Claude's own result stands",
                evidence.detail
            ));
        }
        let target = qf_policy::recovery_target(&ctx);
        drop(ctx);
        let live_label = live
            .as_ref()
            .map(|l| inner.config.label(l))
            .unwrap_or_else(|| "?".into());
        let mut soonest_at = None;
        let (to, retry, reason) = match target {
            RecoveryTarget::None(why) => return no(format!("usage limit confirmed ({}), but {why}", evidence.detail)),
            RecoveryTarget::Capacity(c) => {
                let reason = format!(
                    "{live_label} hit a usage limit ({}); {} at {:.0}% {} ({} old)",
                    evidence.detail,
                    inner.config.label(&c.id),
                    c.pct,
                    c.window,
                    qf_policy::fmt_age(c.age_s)
                );
                (c.id, true, reason)
            }
            RecoveryTarget::Soonest { id, available_at } => {
                let reason = format!(
                    "every approved account is at its limit; {} is available soonest ({}); Claude's own wait continues",
                    inner.config.label(&id),
                    qf_model::time::to_rfc3339(available_at)
                );
                // Claude's own usage-limit behavior takes over; no Quayflow retry or hold.
                soonest_at = Some(available_at);
                (id, false, reason)
            }
        };
        match mode {
            Mode::Automatic => {}
            Mode::Suggest => {
                let s = Suggestion {
                    id: random_hex(6),
                    ts: now,
                    env: env.clone(),
                    from: live.clone(),
                    to: to.clone(),
                    how: SwitchHow::Recovery,
                    reason: reason.clone(),
                };
                let label = inner.config.label(&to);
                inner.envs.get_mut(&env).expect("env").suggestion = Some(s);
                self.emit(
                    &mut inner,
                    "suggestion",
                    Some(&env),
                    &format!("Usage limit reached. Suggest switching to {label}: /quay accept"),
                );
                return RecoverResponse {
                    available_at: soonest_at,
                    ..no(format!(
                        "Suggest mode: no automatic switch; /quay accept switches to {label}"
                    ))
                };
            }
            Mode::Manual => {
                return RecoverResponse {
                    available_at: soonest_at,
                    ..no("Manual mode: no automatic switch")
                }
            }
        }
        drop(inner);
        match self.start_switch(&env, &to, SwitchHow::Recovery, &reason, false, true) {
            Ok(t) if t.state.written() => {
                let mut inner = self.lock();
                // This session's next recovery is a new incident and needs genuine progress first.
                inner.progressed.insert(r.session_id.clone(), false);
                RecoverResponse {
                    retry,
                    switched: true,
                    to: Some(to.clone()),
                    to_label: Some(inner.config.label(&to)),
                    why: reason,
                    transition: Some(t.id),
                    available_at: soonest_at,
                }
            }
            Ok(t) => no(format!("switch not made: {}", t.detail.unwrap_or_default())),
            Err(e) if e.code == "conflict" => {
                // Another failure's switch started between our check and our request: share it.
                let tid = {
                    let inner = self.lock();
                    inner
                        .envs
                        .get(&env)
                        .and_then(|st| st.transition.as_ref().filter(|t| shareable(t.how)))
                        .map(|t| t.id.clone())
                };
                let Some(tid) = tid else {
                    return no(format!("switch not made: {}", e.message));
                };
                match self.wait_transition(&env, &tid, concurrent_wait(&tuning)) {
                    Some(t) if t.state.written() => {
                        let inner = self.lock();
                        let live = inner.envs.get(&env).and_then(|s| s.live.clone());
                        if live.as_ref() != Some(&t.to) {
                            return no("the concurrent switch's account is no longer live");
                        }
                        self.share(&inner, r, &env, &t, &pcts)
                    }
                    Some(t) => no(format!(
                        "the switch for a concurrent failure was not made: {}",
                        t.detail.unwrap_or_default()
                    )),
                    None => no("a switch for a concurrent failure is still in progress; Claude's own result stands"),
                }
            }
            Err(e) => no(format!("switch not made: {}", e.message)),
        }
    }

    /// Answer a concurrent failure with the switch already made for another one: retry on its
    /// destination, but only on a confirmed usage limit of the account the failed request used.
    fn share(
        &self,
        inner: &Inner,
        r: &RecoverRequest,
        env: &EnvId,
        t: &Transition,
        pcts: &[(String, f64)],
    ) -> RecoverResponse {
        let Some(mut ctx) = Engine::ctx(inner, env, r.model.as_deref()) else {
            return no("unknown environment");
        };
        // The failed request ran on the switch's source account, not on the new live one.
        ctx.live = t.from.as_ref();
        let evidence = qf_policy::confirm_limit(&ctx, pcts, r.simulated);
        if !evidence.confirmed {
            return no(format!(
                "not a confirmed usage limit ({}); Claude's own result stands",
                evidence.detail
            ));
        }
        RecoverResponse {
            retry: true,
            switched: false,
            to: Some(t.to.clone()),
            to_label: Some(inner.config.label(&t.to)),
            why: format!(
                "already switched for a concurrent failure ({}); retrying on the new account",
                evidence.detail
            ),
            transition: Some(t.id.clone()),
            available_at: None,
        }
    }

    /// Wait up to `bound` for transition `tid` in `env` to leave its pre-write states, without
    /// holding the engine lock. Returns the transition once it is no longer blocking.
    fn wait_transition(&self, env: &EnvId, tid: &str, bound: Duration) -> Option<Transition> {
        let deadline = Instant::now() + bound;
        loop {
            {
                let inner = self.lock();
                let st = inner.envs.get(env)?;
                let current = st.transition.as_ref().filter(|t| t.id == tid);
                match current {
                    Some(_) if st.blocking_transition() => {}
                    Some(t) => return Some(t.clone()),
                    // Replaced by a later transition: the outcome is in the last switch, if written.
                    None => return st.last_switch.clone().filter(|t| t.id == tid),
                }
            }
            if Instant::now() >= deadline || self.shutdown.load(Ordering::SeqCst) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Switches made by automation that a concurrent failure may share (FR-15).
fn shareable(how: SwitchHow) -> bool {
    matches!(
        how,
        SwitchHow::Recovery | SwitchHow::Automatic | SwitchHow::ReturnHome | SwitchHow::ModelFallback
    )
}

/// How long a concurrent failure waits for an in-flight switch: the lock wait plus a margin,
/// within the client's request timeout.
fn concurrent_wait(t: &Tuning) -> Duration {
    Duration::from_secs((t.lock_wait_s + 5).min(50))
}
