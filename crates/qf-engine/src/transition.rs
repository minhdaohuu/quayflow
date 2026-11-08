//! Transitions (FR-11, FR-12) and adoption tracking (FR-13).
//!
//! REQUESTED -> PREPARING -> (IDLE_WAIT) -> APPLYING -> VERIFYING_ADOPTION -> COMPLETED.
//! Before the write, cancel/timeout/lock-held leaves the login unchanged; after the write,
//! uncertainty is reconciled from the real file state, never assumed.

use crate::core::{random_hex, Engine, Inner};
use qf_adapter::SwapError;
use qf_model::api::{AdoptionReport, StepReport, StepResponse};
use qf_model::time::now_ms;
use qf_model::*;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub blocked_by: Option<Box<SessionInfo>>,
}

impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Refusal {
            code,
            message: message.into(),
            blocked_by: None,
        }
    }
}

impl Engine {
    /// Validate and start a switch. `sync` runs it in the caller's thread (recovery);
    /// otherwise a worker thread runs it and the caller gets the requested transition.
    pub fn start_switch(
        self: &Arc<Self>,
        env: &EnvId,
        to: &AccountId,
        how: SwitchHow,
        reason: &str,
        idle_wait: bool,
        sync: bool,
    ) -> Result<Transition, Refusal> {
        let tid;
        {
            let mut inner = self.lock();
            let st = inner
                .envs
                .get(env)
                .ok_or_else(|| Refusal::new("not_found", format!("no environment \"{env}\"")))?;
            if st.blocking_transition() {
                return Err(Refusal::new("conflict", "Transition already started; checking state"));
            }
            let account = inner
                .config
                .account(to)
                .cloned()
                .ok_or_else(|| Refusal::new("not_found", "no registered account with that name"))?;
            if st.live.as_ref() == Some(to) && st.unknown.is_none() {
                return Err(Refusal::new("noop", format!("{} is already live", account.label)));
            }
            // Live there, its credentials there, or a switch to it there not yet ended.
            if let Some((other, _)) = inner.envs.iter().find(|(id, s)| {
                *id != env
                    && (s.holds_login(to)
                        || s.transition
                            .as_ref()
                            .is_some_and(|t| &t.to == to && !t.state.is_terminal()))
            }) {
                return Err(Refusal::new(
                    "refused",
                    format!(
                        "{} is live in environment \"{other}\"; one login cannot be live in two environments (both would refresh the same token)",
                        account.label
                    ),
                ));
            }
            if !self.vault.exists(to.as_str()) {
                return Err(Refusal::new(
                    "refused",
                    format!("{}: protected copy missing; sign in again (/quay add)", account.label),
                ));
            }
            // Permissions: every running session's group, managed login, health, plan. The plan
            // must serve the model of every session running in this environment.
            let models = running_models(&inner, env);
            let checks: Vec<Option<&str>> = if models.is_empty() {
                vec![None]
            } else {
                models.iter().map(|m| Some(m.as_str())).collect()
            };
            for model in checks {
                let ctx = Engine::ctx(&inner, env, model).expect("env");
                if let Err(ex) = ctx.permitted(to) {
                    let blocker = ctx.blocker(to).cloned();
                    let mut r = Refusal::new("refused", format!("{}: {}", account.label, ex.explain()));
                    if blocker.is_some() {
                        r.code = "conflict";
                        r.message.push_str(
                            ". Move the blocking session into a lane with full history (/quay > Session actions, from that session) so the others can switch.",
                        );
                    }
                    r.blocked_by = blocker.map(Box::new);
                    return Err(r);
                }
            }
            let st = inner.envs.get(env).expect("env");
            // The canary is how a disabled version is checked again, so it is not refused here.
            if !st.in_place.allowed && how != SwitchHow::Canary {
                return Err(Refusal::new(
                    "refused",
                    format!(
                        "in-place switching is disabled here: {}. Continue in a lane with full history instead (/quay > Session actions).",
                        st.in_place.reason
                    ),
                ));
            }
            let automated = !matches!(how, SwitchHow::Manual | SwitchHow::Accepted | SwitchHow::Canary);
            if automated {
                if let Some(why) = st.pause_reason(&inner.config) {
                    return Err(Refusal::new("refused", format!("automation paused: {why}")));
                }
            }
            let t = Transition {
                id: random_hex(8),
                env: env.clone(),
                from: st.live.clone(),
                to: to.clone(),
                how,
                reason: reason.to_string(),
                state: TransitionState::Requested,
                requested_at: now_ms(),
                written_at: None,
                ended_at: None,
                locked_ms: None,
                detail: None,
                failure: None,
            };
            if !self.record(
                &inner,
                "transition.requested",
                json!({"id": t.id, "env": env, "from": t.from, "to": to, "how": how, "reason": reason}),
                true,
            ) {
                return Err(Refusal::new(
                    "refused",
                    "the journal cannot be written; no transition without a record",
                ));
            }
            tid = t.id.clone();
            let st = inner.envs.get_mut(env).expect("env");
            st.cancel.store(false, Ordering::SeqCst);
            st.transition = Some(t);
            if st.suggestion.as_ref().is_some_and(|s| &s.to == to) {
                st.suggestion = None;
            }
            let text = format!("Switching to {} ({}): {reason}", account.label, how_text(how));
            self.emit(&mut inner, "transition", Some(env), &text);
        }
        if sync {
            self.run_transition(env, &tid, idle_wait);
            let inner = self.lock();
            let t = inner
                .envs
                .get(env)
                .and_then(|s| s.transition.clone())
                .filter(|t| t.id == tid);
            return t.ok_or_else(|| Refusal::new("internal", "transition lost"));
        }
        let (me, env2, tid2) = (self.clone(), env.clone(), tid.clone());
        std::thread::spawn(move || me.run_transition(&env2, &tid2, idle_wait));
        let inner = self.lock();
        Ok(inner
            .envs
            .get(env)
            .and_then(|s| s.transition.clone())
            .expect("transition"))
    }

    fn set_state(
        &self,
        inner: &mut Inner,
        env: &EnvId,
        tid: &str,
        state: TransitionState,
        detail: Option<String>,
    ) -> Option<Transition> {
        let st = inner.envs.get_mut(env)?;
        let t = st.transition.as_mut().filter(|t| t.id == tid)?;
        t.state = state;
        if detail.is_some() {
            t.detail = detail;
        }
        if state.is_terminal() {
            t.ended_at = Some(now_ms());
        }
        Some(t.clone())
    }

    fn end(&self, env: &EnvId, tid: &str, state: TransitionState, detail: String) {
        let mut inner = self.lock();
        let failure = match state {
            TransitionState::Cancelled => "cancelled",
            TransitionState::Uncertain => "post_write",
            _ if detail.contains("lock stayed held") => "lock_held",
            _ if detail.contains("protected copy") => "vault",
            _ => "pre_write",
        };
        if let Some(mut t) = self.set_state(&mut inner, env, tid, state, Some(detail.clone())) {
            t.failure = Some(failure.to_string());
            if let Some(cur) = inner.envs.get_mut(env).and_then(|s| s.transition.as_mut()) {
                cur.failure = Some(failure.to_string());
            }
            inner.envs.get_mut(env).expect("env").last_switch = Some(t.clone());
            self.record(
                &inner,
                "transition.ended",
                json!({"id": tid, "env": env, "state": state, "detail": detail}),
                true,
            );
            let text = match state {
                TransitionState::Cancelled => "Switch cancelled; the login is unchanged.".to_string(),
                TransitionState::Failed => format!("Switch not made; the login is unchanged: {detail}"),
                _ => format!("Switch outcome uncertain: {detail}"),
            };
            self.emit(&mut inner, "transition", Some(env), &text);
        }
    }

    pub fn run_transition(self: &Arc<Self>, env: &EnvId, tid: &str, idle_wait: bool) {
        let (envx, to, cancel, wait_s, lock_wait) = {
            let mut inner = self.lock();
            let Some(t) = self.set_state(&mut inner, env, tid, TransitionState::Preparing, None) else {
                return;
            };
            let st = inner.envs.get(env).expect("env");
            (
                st.env.clone(),
                t.to.clone(),
                st.cancel.clone(),
                inner.config.tuning.idle_wait_s,
                inner.config.tuning.lock_wait_s,
            )
        };
        let target = match self.vault.load(to.as_str()) {
            Ok(l) => {
                self.vault_ok(&to);
                l
            }
            Err(e) => {
                let mut inner = self.lock();
                if let Some(a) = inner.config.account_mut(&to) {
                    a.health = Health::VaultError;
                }
                drop(inner);
                return self.end(
                    env,
                    tid,
                    TransitionState::Failed,
                    format!("protected copy unreadable: {e}"),
                );
            }
        };
        if target.account_hash().as_deref() != Some(to.as_str()) {
            return self.end(
                env,
                tid,
                TransitionState::Failed,
                "protected copy holds a different identity".into(),
            );
        }
        // Optional idle wait: prefer a moment with no busy session (bounded, cancelable).
        if idle_wait && wait_s > 0 {
            {
                let mut inner = self.lock();
                self.set_state(&mut inner, env, tid, TransitionState::IdleWait, None);
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(wait_s);
            while std::time::Instant::now() < deadline {
                if cancel.load(Ordering::SeqCst) {
                    return self.end(
                        env,
                        tid,
                        TransitionState::Cancelled,
                        "cancelled before the write".into(),
                    );
                }
                let busy = qf_adapter::sessions::read_sessions(&envx).iter().any(|s| s.is_busy());
                if !busy {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        // The last cancel check and the move to Applying happen under the engine lock, as does
        // `cancel`: a cancel either lands before (and wins) or sees Applying (and is refused).
        {
            let mut inner = self.lock();
            if cancel.load(Ordering::SeqCst) {
                drop(inner);
                return self.end(
                    env,
                    tid,
                    TransitionState::Cancelled,
                    "cancelled before the write".into(),
                );
            }
            self.set_state(&mut inner, env, tid, TransitionState::Applying, None);
        }
        let _cred = self.cred_lock.lock().unwrap_or_else(|e| e.into_inner());
        // Read the saved login again under the credential lock: an idle-account refresh may
        // have rotated its tokens since it was first read, and only the newest copy is valid.
        let target = match self.vault.load(to.as_str()) {
            Ok(l) if l.account_hash().as_deref() == Some(to.as_str()) => l,
            Ok(_) => {
                drop(_cred);
                return self.end(
                    env,
                    tid,
                    TransitionState::Failed,
                    "protected copy holds a different identity".into(),
                );
            }
            Err(e) => {
                drop(_cred);
                return self.end(
                    env,
                    tid,
                    TransitionState::Failed,
                    format!("protected copy unreadable: {e}"),
                );
            }
        };
        let me = self.clone();
        let (env_id, tid_s) = (env.clone(), tid.to_string());
        // Mixed files must be rewritten even when the account record already names the
        // destination: its credentials are another account's.
        let repair = self.lock().envs.get(env).is_some_and(|s| s.mixed.is_some());
        let result = qf_adapter::swap_with(&envx, &target, Duration::from_secs(lock_wait), repair, move |prev| {
            let mut inner = me.lock();
            if let Some(p) = prev {
                // The outgoing login must be safe before it is overwritten: a failure to store
                // it (or back it up) stops the switch.
                me.preserve_live(&mut inner, &env_id, p)?;
            }
            if !me.record(&inner, "transition.applying", json!({"id": tid_s, "env": env_id}), true) {
                return Err("the journal cannot be written".into());
            }
            Ok(())
        });
        drop(_cred);
        match result {
            Ok(out) => {
                // Nothing written because the destination is already live here: its login in
                // the environment may be fresher than the vault copy, so keep it now.
                if let (false, Some(p)) = (out.changed, out.previous.as_ref()) {
                    let mut inner = self.lock();
                    let mixed = inner.envs.get(env).is_some_and(|s| s.mixed.is_some());
                    if let crate::core::LiveOwner::Registered(h) = self.live_owner(&inner, p) {
                        if h == to && !mixed {
                            if let Err(e) = self.capture(&mut inner, &h, p) {
                                self.log
                                    .log("error", "vault.capture_failed", json!({"account": h, "error": e}), &[]);
                            }
                        }
                    }
                }
                self.after_write(env, tid, out)
            }
            Err(SwapError::Lock(e)) => self.end(env, tid, TransitionState::Failed, e.to_string()),
            Err(e @ (SwapError::PreWrite(_) | SwapError::Read(_))) => {
                self.end(env, tid, TransitionState::Failed, e.to_string())
            }
            Err(SwapError::PostWrite(m)) => {
                // Reconcile from the real state of both files instead of assuming either
                // account: the account record and the credentials.
                let now = qf_adapter::read_login(&envx).ok().flatten();
                let identity = now.as_ref().and_then(|l| l.account_hash()).map(AccountId);
                let creds_are_target = now
                    .as_ref()
                    .is_some_and(|l| l.refresh_token().is_some() && l.refresh_token() == target.refresh_token());
                if creds_are_target && identity.as_ref() != Some(&to) {
                    // Half-written: the destination's credentials under another account record.
                    self.end(env, tid, TransitionState::Uncertain, m);
                    let mut inner = self.lock();
                    self.hold_mixed(&mut inner, env, &to, identity.as_ref());
                    self.record(
                        &inner,
                        "transition.uncertain",
                        json!({"id": tid, "env": env, "mixed": true}),
                        true,
                    );
                } else if creds_are_target && identity.as_ref() == Some(&to) {
                    self.after_write(
                        env,
                        tid,
                        qf_adapter::SwapOutcome {
                            previous: None,
                            locked_ms: 0,
                            changed: true,
                            stamp: qf_adapter::login::credentials_stamp(&envx),
                        },
                    );
                    let mut inner = self.lock();
                    self.set_state(
                        &mut inner,
                        env,
                        tid,
                        TransitionState::VerifyingAdoption,
                        Some(format!("written after a retry check: {m}")),
                    );
                } else {
                    self.end(env, tid, TransitionState::Uncertain, m);
                    let mut inner = self.lock();
                    let tidc = tid.to_string();
                    self.read_live(&mut inner, env, false);
                    self.record(&inner, "transition.uncertain", json!({"id": tidc, "env": env}), true);
                }
            }
        }
    }

    fn after_write(self: &Arc<Self>, env: &EnvId, tid: &str, out: qf_adapter::SwapOutcome) {
        let mut inner = self.lock();
        let now = now_ms();
        let Some(t) = inner
            .envs
            .get(env)
            .and_then(|s| s.transition.clone())
            .filter(|t| t.id == tid)
        else {
            return;
        };
        let pin_reset = inner
            .observations
            .iter()
            .find(|o| o.account == t.to)
            .and_then(|o| o.binding())
            .and_then(|w| w.resets_at);
        let pin_until = qf_policy::pin_until(now, &inner.config.tuning, pin_reset);
        let st = inner.envs.get_mut(env).expect("env");
        st.live = Some(t.to.clone());
        st.unknown = None;
        st.signed_out = false;
        st.own_stamp = out.stamp;
        st.last_stamp = out.stamp;
        st.other_switcher = false;
        st.external_changes.clear();
        // Both files now hold the destination's login: a mixed state or an earlier
        // unresolved switch no longer holds automation.
        st.clear_hold();
        let unpinned = st.unpinned_transition.take().is_some_and(|id| id == t.id);
        st.pin = if t.how.pins() && !unpinned {
            Some(Pin {
                account: t.to.clone(),
                until: pin_until,
                at: now,
            })
        } else {
            None
        };
        if t.how != SwitchHow::Canary {
            st.last_proactive = Some(now);
        }
        let clients: Vec<SessionInfo> = st.sessions.clone();
        st.adoption.retain(|a| a.at.is_none_or(|x| now - x < 3_600_000));
        for s in &clients {
            st.adoption.retain(|a| a.session_id != s.session_id);
            st.adoption.push(ClientAdoption {
                session_id: s.session_id.clone(),
                pid: s.pid,
                transition: tid.to_string(),
                destination: t.to.clone(),
                last_request_account: None,
                state: AdoptionState::PendingNextRequest,
                version: s.version.clone(),
                at: None,
            });
        }
        let state = if clients.is_empty() {
            TransitionState::Completed
        } else {
            TransitionState::VerifyingAdoption
        };
        let tr = st.transition.as_mut().expect("transition");
        tr.state = state;
        tr.written_at = Some(now);
        tr.locked_ms = Some(out.locked_ms);
        if state == TransitionState::Completed {
            tr.ended_at = Some(now);
            tr.detail = Some("no running client; new sessions start on this account".into());
        }
        let tr = tr.clone();
        st.last_switch = Some(tr.clone());
        inner.schedule.poke(&t.to);
        if let Some(f) = &t.from {
            inner.schedule.poke(f);
        }
        self.record(
            &inner,
            "transition.written",
            json!({"id": tid, "env": env, "from": t.from, "to": t.to, "how": t.how, "lockedMs": out.locked_ms, "clients": clients.len()}),
            true,
        );
        let text = format!(
            "Switched to {} ({}). Each session uses it from its next request; requests in progress finish on the previous account.",
            inner.config.label(&t.to),
            how_text(t.how)
        );
        self.emit(&mut inner, "switched", Some(env), &text);
    }

    pub fn cancel(&self, tid: &str) -> Result<String, String> {
        let inner = self.lock();
        for st in inner.envs.values() {
            if let Some(t) = st.transition.as_ref().filter(|t| t.id == tid) {
                return match t.state {
                    TransitionState::Requested | TransitionState::Preparing | TransitionState::IdleWait => {
                        st.cancel.store(true, Ordering::SeqCst);
                        Ok("Cancellation requested".into())
                    }
                    s if s.is_terminal() => Ok("This transition has already ended".into()),
                    _ => Ok("Transition already started; checking state".into()),
                };
            }
        }
        Err("no such transition".into())
    }

    /// After every request: whether this client should report the identity its request used.
    pub fn step(&self, r: &StepReport) -> StepResponse {
        let mut inner = self.lock();
        if let Some(m) = &r.model {
            inner.session_model.insert(r.session_id.clone(), m.clone());
        }
        if r.ok {
            inner.progressed.insert(r.session_id.clone(), true);
        }
        let env = self.env_of_session(&inner, &r.session_id);
        let pending = inner
            .envs
            .get(&env)
            .and_then(|st| {
                st.adoption
                    .iter()
                    .find(|a| a.session_id == r.session_id && a.state == AdoptionState::PendingNextRequest)
            })
            .map(|a| a.transition.clone());
        // A request that started before the switch was written was in flight: it finishes on
        // the previous account by design and says nothing about adoption.
        let in_flight = match (&pending, r.started_at) {
            (Some(tid), Some(started)) => self
                .written_at(&inner, &env, tid)
                .is_some_and(|written| started < written),
            _ => false,
        };
        let pending = pending.filter(|_| !in_flight);
        if let Some(c) = inner.canary.as_mut() {
            c.steps.push(json!({"ts": now_ms(), "startedAt": r.started_at, "session": r.session_id, "index": r.index, "agent": r.agent_id, "stopReason": r.stop_reason, "ok": r.ok, "inFlight": in_flight}));
        }
        // The session's rate limits describe the account its request used: fold them in only
        // when that is known to be the live account (no adoption pending for this client).
        if pending.is_none() && !in_flight && r.ok && !r.rate_limits.is_empty() {
            let live = inner
                .envs
                .get(&env)
                .and_then(|s| s.live.clone().filter(|_| s.unknown.is_none()));
            if let Some(live) = live {
                self.session_observation(&mut inner, &live, &r.rate_limits);
            }
        }
        let canary = inner.canary.is_some();
        StepResponse {
            verify: (pending.is_some() && r.ok) || canary,
            transition: pending,
        }
    }

    /// A client's report of the identity its first request after a write used.
    pub fn adoption(&self, r: &AdoptionReport) -> Result<String, String> {
        let mut inner = self.lock();
        let Some(account) = r.account.clone() else {
            return Ok("no identity evidence; still pending".into());
        };
        inner.verified.insert(r.session_id.clone(), (account.clone(), now_ms()));
        if let Some(c) = inner.canary.as_mut() {
            c.steps.push(json!({"ts": now_ms(), "startedAt": r.started_at, "session": r.session_id, "account": account, "adoption": true}));
        }
        let env = self.env_of_session(&inner, &r.session_id);
        // Identity of a request that was already in flight at the write: not adoption evidence.
        let pending_tid = inner.envs.get(&env).and_then(|st| {
            st.adoption
                .iter()
                .find(|a| a.session_id == r.session_id && a.state == AdoptionState::PendingNextRequest)
                .map(|a| a.transition.clone())
        });
        if let (Some(tid), Some(started)) = (&pending_tid, r.started_at) {
            if self.written_at(&inner, &env, tid).is_some_and(|w| started < w) {
                return Ok("request was in flight at the switch; still pending".into());
            }
        }
        // A session that reads another Keychain item cannot follow the switch: that is about how
        // it was started, not about the Claude version. Stop waiting for it.
        if let Some(why) = self.keychain_mismatch(&inner, &r.session_id) {
            if let Some(st) = inner.envs.get_mut(&env) {
                st.adoption
                    .retain(|a| !(a.session_id == r.session_id && a.state == AdoptionState::PendingNextRequest));
            }
            return Ok(format!("not adoption evidence: {why}"));
        }
        let Some(st) = inner.envs.get_mut(&env) else {
            return Ok("unknown environment".into());
        };
        let Some(a) = st
            .adoption
            .iter_mut()
            .find(|a| a.session_id == r.session_id && a.state == AdoptionState::PendingNextRequest)
        else {
            return Ok("nothing pending".into());
        };
        a.last_request_account = Some(account.clone());
        a.at = Some(now_ms());
        let ok = a.destination == account;
        a.state = if ok {
            AdoptionState::Adopted
        } else {
            AdoptionState::Mismatch
        };
        let (tid, version, dest) = (a.transition.clone(), a.version.clone(), a.destination.clone());
        self.record(
            &inner,
            "adoption",
            json!({"session": r.session_id, "transition": tid, "destination": dest, "used": account, "ok": ok, "version": version}),
            true,
        );
        if !ok {
            // Feeds the version check: in-place switching stops for this version (FR-13, FR-24).
            if let Some(v) = version {
                let rec = inner
                    .compat
                    .entry(v.clone())
                    .or_insert_with(|| crate::core::static_check(&v));
                rec.live_pass = Some(false);
                rec.live_detail = Some(format!(
                    "session {} used #{} after a switch to #{}",
                    short(&r.session_id),
                    account,
                    dest
                ));
                let rec = rec.clone();
                self.save_compat(&rec);
            }
            let text = format!(
                "This session's request did not use {} after the switch. In-place switching is disabled for this Claude version; review the full-history fallback in /quay (Session actions) to continue on the intended account.",
                inner.config.label(&dest)
            );
            self.emit(&mut inner, "notice", Some(&env), &text);
        } else {
            let text = format!(
                "Session {} now runs on {} (verified on its request).",
                short(&r.session_id),
                inner.config.label(&dest)
            );
            self.emit(&mut inner, "adoption", Some(&env), &text);
            if let Some(v) = inner
                .envs
                .get(&env)
                .and_then(|s| s.sessions.iter().find(|x| x.session_id == r.session_id))
                .and_then(|s| s.version.clone())
            {
                if let Some(rec) = inner.compat.get_mut(&v) {
                    if rec.live_pass.is_none() {
                        rec.live_pass = Some(true);
                        rec.live_detail = Some("first real switch adopted at the next request".into());
                        let rec = rec.clone();
                        self.save_compat(&rec);
                    }
                }
            }
        }
        self.finish_adoption(&mut inner, &env);
        Ok(if ok { "adopted".into() } else { "mismatch".into() })
    }
}

impl Engine {
    /// When transition `tid` wrote the login in `env`, if known.
    fn written_at(&self, inner: &Inner, env: &EnvId, tid: &str) -> Option<i64> {
        let st = inner.envs.get(env)?;
        [st.transition.as_ref(), st.last_switch.as_ref()]
            .into_iter()
            .flatten()
            .find(|t| t.id == tid)
            .and_then(|t| t.written_at)
    }
}

/// Models last reported by the sessions currently running in `env` (sorted, distinct).
pub fn running_models(inner: &Inner, env: &EnvId) -> Vec<String> {
    let mut models: Vec<String> = inner
        .envs
        .get(env)
        .map(|st| {
            st.sessions
                .iter()
                .filter_map(|s| inner.session_model.get(&s.session_id).cloned())
                .collect()
        })
        .unwrap_or_default();
    models.sort();
    models.dedup();
    models
}

/// The first 8 characters of `s` (char-safe).
pub fn short(s: &str) -> &str {
    match s.char_indices().nth(8) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

pub fn how_text(h: SwitchHow) -> &'static str {
    match h {
        SwitchHow::Manual => "your choice, pinned",
        SwitchHow::Accepted => "suggestion accepted",
        SwitchHow::Automatic => "automatic",
        SwitchHow::ReturnHome => "returning home",
        SwitchHow::Recovery => "usage-limit recovery",
        SwitchHow::ModelFallback => "model unavailable; switched back",
        SwitchHow::Canary => "version canary",
    }
}
