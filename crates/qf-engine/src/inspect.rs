//! Read-only views the Mod's review screens draw from: the inputs of a decision, the full
//! identity of a login (local review only), a full-history move named before it is
//! committed, and the journal trail of recent operations. Plus two small settings the
//! review screens change: a pin on the live account and the proactive cooldown / pin length.

use crate::accounts::ApiErr;
use crate::core::Engine;
use qf_model::api::{CandidateView, ExplainView, HandoffCheck, IdentityView, OperationView};
use qf_model::time::now_ms;
use qf_model::*;
use qf_policy::Exclusion;
use serde_json::{json, Value};

/// Whether a window limits requests of `model` (5h, 7d and the model's own weekly window).
fn applies(w: &Window, model: Option<&str>) -> bool {
    match (w.key.strip_prefix("7d:"), model) {
        (Some(m), Some(model)) => model_matches(m, model),
        _ => true,
    }
}

fn identity_of(login: &qf_vault::Login) -> IdentityView {
    let s = |k: &str| {
        login
            .account
            .get(k)
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|x| !x.trim().is_empty())
    };
    IdentityView {
        account: AccountId::from_uuid(login.account_uuid().unwrap_or_default()),
        kind: login.kind(),
        subscription: login.subscription().map(String::from),
        display: s("emailAddress").or_else(|| s("displayName")),
        org_name: login.org_name(),
    }
}

impl Engine {
    /// The decision inputs of `env` for the "Why this account?" review: every account's
    /// status (live, eligible, excluded, unknown) with its reason and observation age, the
    /// threshold, margin and cooldown, and the home account's state. Read-only.
    pub fn explain(&self, env: &EnvId, session_id: Option<&str>) -> Result<ExplainView, ApiErr> {
        let inner = self.lock();
        let st = inner
            .envs
            .get(env)
            .ok_or_else(|| ApiErr::not_found(format!("no environment \"{env}\"")))?;
        let model = session_id.and_then(|s| inner.session_model.get(s)).map(String::as_str);
        let ctx = Engine::ctx(&inner, env, model).ok_or_else(|| ApiErr::not_found("no environment"))?;
        let cfg = &inner.config;
        let now = now_ms();
        let threshold = qf_policy::effective_threshold(&ctx.governing);
        let margin = qf_policy::effective_margin(&ctx.governing);
        let hard = cfg.tuning.hard_limit_pct;
        let cooldown_left_s = st
            .last_proactive
            .map(|t| cfg.tuning.cooldown_s as i64 * 1000 - (now - t))
            .filter(|left| *left > 0)
            .map(|left| (left + 999) / 1000)
            .unwrap_or(0);
        let explained = qf_policy::explain_all(&ctx);
        let mut candidates = vec![];
        for a in &cfg.accounts {
            let obs = ctx.observation(&a.id);
            let w = obs.and_then(|o| o.binding_for(model));
            let fresh = ctx.fresh_binding(&a.id).is_ok();
            let exhausted = obs.is_some_and(|o| o.windows.iter().any(|w| w.pct >= hard && applies(w, model)));
            let available_at = obs
                .filter(|_| exhausted)
                .and_then(|o| qf_policy::available_at_for(o, hard, now, model));
            let result = explained.iter().find(|(id, _)| id == &a.id).map(|(_, r)| r.clone());
            let (status, reason) = match result {
                Some(Ok(why)) => ("eligible", why),
                Some(Err(Exclusion::IsLive)) => (
                    "live",
                    match (obs, w) {
                        (Some(o), Some(w)) => format!(
                            "live here: {:.0}% {} ({} old)",
                            w.pct,
                            w.key,
                            qf_policy::fmt_age(o.age_ms(now) / 1000)
                        ),
                        _ => "live here; usage not observed yet".into(),
                    },
                ),
                Some(Err(e @ (Exclusion::NoObservation | Exclusion::Stale { .. }))) => ("unknown", e.explain()),
                Some(Err(e)) => ("excluded", e.explain()),
                None => ("unknown", "not evaluated".into()),
            };
            candidates.push(CandidateView {
                account: a.id.clone(),
                status: status.into(),
                reason,
                pct: w.map(|w| w.pct),
                window: w.map(|w| w.key.clone()),
                age_s: obs.map(|o| o.age_ms(now) / 1000),
                fresh,
                available_at,
                exhausted,
                endpoint_note: inner.schedule.note.get(a.id.as_str()).cloned(),
                backoff_s: inner.schedule.backoff_ms.get(a.id.as_str()).map(|b| b / 1000),
                next_check_at: inner.schedule.next_at.get(a.id.as_str()).copied(),
            });
        }
        let live = ctx.live.cloned();
        let pinned = qf_policy::pin_holds(&ctx);
        let home = qf_policy::effective_home(&ctx.governing).cloned();
        let label = |id: &AccountId| cfg.label(id);
        let (home_state, home_detail) = match &home {
            None => (
                "none",
                "No home account: Quayflow stays on the current account until a normal trigger.".to_string(),
            ),
            Some(h) if live.as_ref() == Some(h) => ("home", format!("{} is the home account and is live.", label(h))),
            Some(h) => {
                let room =
                    ctx.permitted(h).is_ok() && ctx.fresh_binding(h).is_ok_and(|(_, w)| w.pct <= threshold - margin);
                let usage = match ctx.fresh_binding(h) {
                    Ok((o, w)) => format!(
                        "{:.0}% {} ({} old)",
                        w.pct,
                        w.key,
                        qf_policy::fmt_age(o.age_ms(now) / 1000)
                    ),
                    Err(e) => e.explain(),
                };
                if !room {
                    (
                        "away",
                        format!(
                            "Away from home: {} is not ready to return to ({usage}; needs fresh usage at or below {:.0}%).",
                            label(h),
                            threshold - margin
                        ),
                    )
                } else if pinned {
                    (
                        "deferred_pin",
                        format!(
                            "{} has room again ({usage}); the return waits until your pin ends.",
                            label(h)
                        ),
                    )
                } else if cooldown_left_s > 0 {
                    (
                        "deferred_cooldown",
                        format!(
                            "{} has room again ({usage}); the return waits for the cooldown ({cooldown_left_s} s).",
                            label(h)
                        ),
                    )
                } else {
                    ("ready", format!("{} has room again ({usage}).", label(h)))
                }
            }
        };
        let live_exhausted = live
            .as_ref()
            .and_then(|l| candidates.iter().find(|c| &c.account == l))
            .is_some_and(|c| c.exhausted);
        let (all_exhausted, soonest) = if live_exhausted {
            match qf_policy::recovery_target(&ctx) {
                qf_policy::RecoveryTarget::Capacity(_) => (false, None),
                qf_policy::RecoveryTarget::Soonest { id, .. } => (true, Some(id)),
                qf_policy::RecoveryTarget::None(_) => (true, None),
            }
        } else {
            (false, None)
        };
        Ok(ExplainView {
            env: env.clone(),
            mode: qf_policy::effective_mode(&ctx.governing),
            governing_groups: ctx.governing.iter().map(|g| g.name.clone()).collect(),
            threshold,
            margin,
            cooldown_s: cfg.tuning.cooldown_s,
            cooldown_left_s,
            hard_limit_pct: hard,
            live,
            home,
            home_state: home_state.into(),
            home_detail,
            pinned,
            all_exhausted,
            soonest,
            decision: st.decision.clone(),
            candidates,
        })
    }

    /// The full identity of the live login of `env`, or of a registered account's protected
    /// copy, for local review. Nothing is stored or logged.
    pub fn identity(&self, env: Option<&EnvId>, account: Option<&str>) -> Result<IdentityView, ApiErr> {
        if let Some(key) = account.filter(|a| !a.is_empty()) {
            let id = {
                let inner = self.lock();
                self.find_account(&inner, key)?
            };
            let login = self
                .vault
                .load(id.as_str())
                .map_err(|e| ApiErr::refused(format!("the protected copy cannot be read: {e}")))?;
            return Ok(identity_of(&login));
        }
        let env_id = env.cloned().unwrap_or_else(|| self.default_env.id.clone());
        let envx = self
            .lock()
            .envs
            .get(&env_id)
            .map(|s| s.env.clone())
            .ok_or_else(|| ApiErr::not_found(format!("no environment \"{env_id}\"")))?;
        let login = qf_adapter::read_login(&envx)
            .map_err(ApiErr::invalid)?
            .ok_or_else(|| ApiErr::refused("no login in this environment; sign in with Claude first"))?;
        Ok(identity_of(&login))
    }

    /// What moving `session_id` into `to_env` with full history involves, before it is done.
    pub fn handoff_check(&self, session_id: &str, to_env: &str) -> Result<HandoffCheck, ApiErr> {
        let inner = self.lock();
        let to_id = if to_env.is_empty() {
            self.default_env.id.clone()
        } else {
            EnvId(to_env.into())
        };
        let to = inner
            .envs
            .get(&to_id)
            .ok_or_else(|| ApiErr::not_found(format!("no environment \"{to_id}\"")))?;
        let to_login = to.live.clone().filter(|_| to.unknown.is_none());
        let source = inner.envs.iter().find_map(|(id, st)| {
            st.sessions
                .iter()
                .find(|s| s.session_id == session_id)
                .map(|s| (id.clone(), s.pid))
        });
        let (shell, shell_supported) = match &source {
            Some((_, pid)) => match qf_adapter::process::powershell_ancestor(*pid) {
                Some((_, name)) => (Some(name), true),
                None => (
                    qf_adapter::process::parent_pid(*pid).and_then(qf_adapter::process::process_name),
                    false,
                ),
            },
            None => (None, false),
        };
        let blocked = if to_login.is_none() {
            Some(format!(
                "environment \"{to_id}\" has no registered login; switch an account into it first"
            ))
        } else if source.as_ref().is_some_and(|(id, _)| id == &to_id) {
            Some("the session already runs in that environment".into())
        } else {
            None
        };
        Ok(HandoffCheck {
            to_env: to_id,
            to_login,
            running: source.is_some(),
            shell,
            shell_supported,
            prompt_hook: inner.config.consents.prompt_hook.is_some(),
            blocked,
        })
    }

    /// Recent transitions, newest first, each with its journal trail.
    pub fn operations(&self, limit: usize) -> Vec<OperationView> {
        let mut ops: Vec<OperationView> = vec![];
        for rec in self.journal.read_all() {
            let event = rec["event"].as_str().unwrap_or("").to_string();
            let id = if event.starts_with("transition.") {
                rec["id"].as_str()
            } else if event == "adoption" {
                rec["transition"].as_str()
            } else {
                None
            };
            let Some(id) = id.filter(|i| !i.is_empty()).map(String::from) else {
                continue;
            };
            let s = |k: &str| rec[k].as_str().map(String::from);
            let stage = match event.strip_prefix("transition.") {
                Some("ended") => format!("ended: {}", rec["state"].as_str().unwrap_or("?")),
                Some(x) => x.to_string(),
                None => format!(
                    "client {}",
                    if rec["ok"].as_bool() == Some(true) {
                        "adopted"
                    } else {
                        "mismatch"
                    }
                ),
            };
            match ops.iter_mut().find(|o| o.id == id) {
                Some(op) => {
                    op.env = op.env.take().or_else(|| s("env"));
                    op.how = op.how.take().or_else(|| s("how"));
                    op.from = op.from.take().or_else(|| s("from"));
                    op.to = op.to.take().or_else(|| s("to"));
                    op.stage = stage;
                    op.records.push(rec);
                }
                None => ops.push(OperationView {
                    id,
                    ts: rec["ts"].as_i64().unwrap_or(0),
                    env: s("env"),
                    how: s("how"),
                    from: s("from"),
                    to: s("to"),
                    stage,
                    records: vec![rec],
                }),
            }
        }
        ops.sort_by_key(|o| std::cmp::Reverse(o.ts));
        ops.truncate(limit);
        ops
    }

    /// Pin the live account of `env` (a manual choice without a switch): the same pin a
    /// manual switch sets, released by unpin, the pin period, its window reset or the
    /// pin-release level.
    pub fn pin_live(&self, env: &EnvId) -> Result<Pin, ApiErr> {
        let mut inner = self.lock();
        let now = now_ms();
        let st = inner
            .envs
            .get(env)
            .ok_or_else(|| ApiErr::not_found(format!("no environment \"{env}\"")))?;
        let live = st
            .live
            .clone()
            .filter(|_| st.unknown.is_none() && st.mixed.is_none())
            .ok_or_else(|| ApiErr::refused("no registered login is live here to pin"))?;
        let reset = inner
            .observations
            .iter()
            .find(|o| o.account == live)
            .and_then(|o| o.binding())
            .and_then(|w| w.resets_at);
        let pin = Pin {
            account: live.clone(),
            until: qf_policy::pin_until(now, &inner.config.tuning, reset),
            at: now,
        };
        inner.envs.get_mut(env).expect("env").pin = Some(pin.clone());
        self.record(&inner, "pin", json!({"env": env, "account": live}), false);
        let text = format!("Pinned {} here.", inner.config.label(&live));
        self.emit(&mut inner, "state", Some(env), &text);
        Ok(pin)
    }

    /// Proactive cooldown (0-3600 s) and manual pin length (5-1440 min), for every group.
    pub fn set_tuning(&self, body: &Value) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        let before = inner.config.tuning.clone();
        if let Some(v) = body.get("cooldownS") {
            let s = v
                .as_u64()
                .filter(|s| *s <= 3600)
                .ok_or_else(|| ApiErr::invalid("cooldownS is 0-3600 seconds"))?;
            inner.config.tuning.cooldown_s = s;
        }
        if let Some(v) = body.get("pinMinutes") {
            let m = v
                .as_u64()
                .filter(|m| (5..=1440).contains(m))
                .ok_or_else(|| ApiErr::invalid("pinMinutes is 5-1440 minutes"))?;
            inner.config.tuning.pin_minutes = m;
        }
        if let Err(e) = self.save_config(&inner) {
            inner.config.tuning = before;
            return Err(ApiErr::invalid(e));
        }
        let t = &inner.config.tuning;
        let msg = format!(
            "Cooldown {} s after a proactive switch; manual pins last {} min.",
            t.cooldown_s, t.pin_minutes
        );
        self.record(
            &inner,
            "tuning",
            json!({"cooldownS": t.cooldown_s, "pinMinutes": t.pin_minutes}),
            false,
        );
        self.emit(&mut inner, "state", None, &msg);
        Ok(msg)
    }
}
