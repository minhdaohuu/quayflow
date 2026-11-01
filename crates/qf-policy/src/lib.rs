//! Pure decision logic: eligibility, ranking, proactive trigger, pin, return home,
//! hard-limit confirmation, recovery target and soonest-available selection.
//! No I/O; every input comes in through [`Ctx`].

use qf_model::{
    Account, AccountId, Group, Health, ManagedLogin, Mode, Observation, Pin, Rank, SessionInfo, Tuning, Window,
};

/// Everything a decision needs, for one credential environment.
#[derive(Debug, Clone)]
pub struct Ctx<'a> {
    pub now: i64,
    pub accounts: &'a [Account],
    /// Groups that govern the environment (running sessions' groups, or the default).
    pub governing: Vec<&'a Group>,
    /// Running sessions with their resolved group, for naming a blocker.
    pub sessions: &'a [SessionInfo],
    /// All groups (to look up a blocking session's group).
    pub all_groups: &'a [Group],
    pub observations: &'a [Observation],
    pub live: Option<&'a AccountId>,
    pub managed: &'a ManagedLogin,
    pub tuning: &'a Tuning,
    pub pin: Option<&'a Pin>,
    /// Time of the last proactive switch in this environment.
    pub last_proactive_switch: Option<i64>,
    /// Session's current model, when known (plan compatibility, per-model windows).
    pub model: Option<&'a str>,
    /// Another operation (transition) is pending in the environment.
    pub busy_transition: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Exclusion {
    NotInGroup { group: String },
    ManagedLogin,
    GroupConflict { session: String, group: String },
    Health(Health),
    PlanIncompatible { model: String },
    NoObservation,
    Stale { age_s: i64 },
    AtLimit { pct: f64, window: String },
    NoHeadroom { pct: f64, window: String, needed: f64 },
    IsLive,
}

impl Exclusion {
    pub fn explain(&self) -> String {
        match self {
            Exclusion::NotInGroup { group } => format!("not in group \"{group}\""),
            Exclusion::ManagedLogin => {
                "refused by this environment's managed login enforcement (your organization's administrator controls it)".into()
            }
            Exclusion::GroupConflict { session, group } => {
                format!("session {session} works in group \"{group}\", which does not include this account")
            }
            Exclusion::Health(Health::StaleAccess) => "access token expired while idle; usage unknown until it is live".into(),
            Exclusion::Health(Health::RefreshRejected) => "sign-in expired (refresh rejected); reauthenticate".into(),
            Exclusion::Health(Health::VaultError) => "protected copy cannot be read".into(),
            Exclusion::Health(Health::Ok) => "ok".into(),
            Exclusion::PlanIncompatible { model } => format!("plan cannot serve model {model}"),
            Exclusion::NoObservation => "usage not observed yet".into(),
            Exclusion::Stale { age_s } => format!("usage observation {} old (stale is never zero)", fmt_age(*age_s)),
            Exclusion::AtLimit { pct, window } => format!("at its limit ({pct:.0}% {window})"),
            Exclusion::NoHeadroom { pct, window, needed } => {
                format!("{pct:.0}% {window}; needs {needed:.0}% or less to be worth a switch")
            }
            Exclusion::IsLive => "already live".into(),
        }
    }
}

pub fn fmt_age(s: i64) -> String {
    if s < 90 {
        format!("{s} s")
    } else if s < 5400 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

/// The effective mode: most restrictive of the governing groups.
pub fn effective_mode(groups: &[&Group]) -> Mode {
    groups.iter().map(|g| g.mode).min().unwrap_or(Mode::Suggest)
}

pub fn effective_threshold(groups: &[&Group]) -> f64 {
    groups
        .iter()
        .map(|g| g.threshold)
        .fold(f64::INFINITY, f64::min)
        .min(100.0)
}

pub fn effective_margin(groups: &[&Group]) -> f64 {
    groups.iter().map(|g| g.margin).fold(0.0, f64::max)
}

/// Home account: only when every governing group names the same Preferred account.
pub fn effective_home<'a>(groups: &[&'a Group]) -> Option<&'a AccountId> {
    let first = groups.first()?.preferred.as_ref()?;
    groups
        .iter()
        .all(|g| g.preferred.as_ref() == Some(first))
        .then_some(first)
}

impl<'a> Ctx<'a> {
    pub fn account(&self, id: &AccountId) -> Option<&'a Account> {
        self.accounts.iter().find(|a| &a.id == id)
    }

    pub fn observation(&self, id: &AccountId) -> Option<&'a Observation> {
        self.observations.iter().find(|o| &o.account == id)
    }

    fn fresh_limit_s(&self, id: &AccountId) -> i64 {
        if self.live == Some(id) {
            self.tuning.live_fresh_s as i64
        } else {
            self.tuning.idle_fresh_s as i64
        }
    }

    /// Fresh binding window of an account, or the reason there is none.
    pub fn fresh_binding(&self, id: &AccountId) -> Result<(&'a Observation, &'a Window), Exclusion> {
        let o = self.observation(id).ok_or(Exclusion::NoObservation)?;
        let age = o.age_ms(self.now) / 1000;
        if age > self.fresh_limit_s(id) {
            return Err(Exclusion::Stale { age_s: age });
        }
        let w = o.binding_for(self.model).ok_or(Exclusion::NoObservation)?;
        Ok((o, w))
    }

    /// Permission-level checks shared by every kind of switch (not capacity).
    pub fn permitted(&self, id: &AccountId) -> Result<&'a Account, Exclusion> {
        let account = self.account(id).ok_or(Exclusion::NotInGroup { group: "?".into() })?;
        for g in &self.governing {
            if !g.contains(id) {
                // Name the session whose group refuses it, when one exists (FR-06 conflicts).
                if let Some(s) = self
                    .sessions
                    .iter()
                    .find(|s| s.group.as_deref() == Some(g.name.as_str()))
                {
                    if self.governing.len() > 1 {
                        return Err(Exclusion::GroupConflict {
                            session: s.display(),
                            group: g.name.clone(),
                        });
                    }
                }
                return Err(Exclusion::NotInGroup { group: g.name.clone() });
            }
        }
        if !self.managed.permits(account) {
            return Err(Exclusion::ManagedLogin);
        }
        match account.health {
            Health::RefreshRejected | Health::VaultError => return Err(Exclusion::Health(account.health)),
            _ => {}
        }
        if let Some(model) = self.model {
            if account.unavailable_models.iter().any(|m| m.eq_ignore_ascii_case(model)) {
                return Err(Exclusion::PlanIncompatible { model: model.into() });
            }
        }
        Ok(account)
    }

    /// The blocking session for a destination, if a running session's group refuses it.
    pub fn blocker(&self, id: &AccountId) -> Option<&'a SessionInfo> {
        self.sessions.iter().find(|s| {
            s.group
                .as_deref()
                .and_then(|g| self.all_groups.iter().find(|x| x.name == g))
                .is_some_and(|g| !g.contains(id))
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: AccountId,
    pub pct: f64,
    pub window: String,
    pub rank: Rank,
    pub age_s: i64,
}

/// Rank candidates: lowest binding first; within 10 points of the best, rank decides.
pub fn rank_candidates(mut c: Vec<Candidate>) -> Vec<Candidate> {
    c.sort_by(|a, b| a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal));
    let Some(best) = c.first().map(|x| x.pct) else { return c };
    let (mut close, rest): (Vec<Candidate>, Vec<Candidate>) = c.into_iter().partition(|x| x.pct - best <= 10.0);
    close.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal))
    });
    close.extend(rest);
    close
}

fn rank_in(groups: &[&Group], id: &AccountId) -> Rank {
    groups.iter().map(|g| g.rank_of(id)).max().unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Proactive {
    /// Nothing to do, with the explanation shown in /quay.
    None(String),
    Switch {
        to: AccountId,
        reason: String,
        return_home: bool,
    },
}

/// Per-account explanation used by the Overview (eligible or why not).
pub fn explain_all(ctx: &Ctx) -> Vec<(AccountId, Result<String, Exclusion>)> {
    let threshold = effective_threshold(&ctx.governing);
    let margin = effective_margin(&ctx.governing);
    ctx.accounts
        .iter()
        .map(|a| {
            let r = (|| {
                if ctx.live == Some(&a.id) {
                    return Err(Exclusion::IsLive);
                }
                ctx.permitted(&a.id)?;
                let (o, w) = ctx.fresh_binding(&a.id)?;
                if w.pct > threshold - margin {
                    return Err(Exclusion::NoHeadroom {
                        pct: w.pct,
                        window: w.key.clone(),
                        needed: threshold - margin,
                    });
                }
                Ok(format!(
                    "{:.0}% {} ({} old)",
                    w.pct,
                    w.key,
                    fmt_age(o.age_ms(ctx.now) / 1000)
                ))
            })();
            (a.id.clone(), r)
        })
        .collect()
}

fn candidates(ctx: &Ctx, max_pct: f64) -> Vec<Candidate> {
    let mut out = vec![];
    for g in ctx.governing.first().map(|g| g.accounts.as_slice()).unwrap_or(&[]) {
        if ctx.live == Some(g) || ctx.permitted(g).is_err() {
            continue;
        }
        if let Ok((o, w)) = ctx.fresh_binding(g) {
            if w.pct <= max_pct {
                out.push(Candidate {
                    id: g.clone(),
                    pct: w.pct,
                    window: w.key.clone(),
                    rank: rank_in(&ctx.governing, g),
                    age_s: o.age_ms(ctx.now) / 1000,
                });
            }
        }
    }
    rank_candidates(out)
}

/// Whether a pin still holds (FR-07): before expiry, while the pinned account is live,
/// and while it is below the pin-release level.
pub fn pin_holds(ctx: &Ctx) -> bool {
    let Some(pin) = ctx.pin else { return false };
    if ctx.now >= pin.until || ctx.live != Some(&pin.account) {
        return false;
    }
    match ctx.fresh_binding(&pin.account) {
        Ok((_, w)) => w.pct < ctx.tuning.pin_release_pct,
        Err(_) => true,
    }
}

/// Pin expiry for a manual switch: pin period or the account's binding reset, whichever is first.
pub fn pin_until(now: i64, tuning: &Tuning, binding_reset: Option<i64>) -> i64 {
    let by_time = now + tuning.pin_minutes as i64 * 60_000;
    match binding_reset {
        Some(r) if r > now => by_time.min(r),
        _ => by_time,
    }
}

/// Proactive decision (threshold trigger or return home). Mode handling is the caller's:
/// Suggest records it, Automatic switches, Manual ignores it.
pub fn proactive(ctx: &Ctx) -> Proactive {
    if ctx.governing.is_empty() {
        return Proactive::None("no group governs this environment".into());
    }
    if ctx.busy_transition {
        return Proactive::None("a transition is in progress".into());
    }
    let Some(live) = ctx.live else {
        return Proactive::None("no registered login is live".into());
    };
    if let Err(e) = ctx.permitted(live) {
        // The live account itself is outside the governing groups: never auto-move on that basis.
        return Proactive::None(format!("live account: {}", e.explain()));
    }
    let threshold = effective_threshold(&ctx.governing);
    let margin = effective_margin(&ctx.governing);
    let cooldown_left = ctx
        .last_proactive_switch
        .map(|t| ctx.tuning.cooldown_s as i64 * 1000 - (ctx.now - t))
        .filter(|left| *left > 0);
    let live_label = ctx.account(live).map(|a| a.label.as_str()).unwrap_or("?");

    let (live_obs, live_w) = match ctx.fresh_binding(live) {
        Ok(v) => v,
        Err(e) => return Proactive::None(format!("{live_label}: {}; not deciding on unknown data", e.explain())),
    };

    if pin_holds(ctx) {
        let until = ctx.pin.map(|p| qf_model::time::to_rfc3339(p.until)).unwrap_or_default();
        return Proactive::None(format!(
            "{live_label} at {:.0}% {}, pinned by you until {until} (/quay unpin)",
            live_w.pct, live_w.key
        ));
    }

    // Return home first: the Preferred account has room again.
    if let Some(home) = effective_home(&ctx.governing) {
        if home != live && ctx.permitted(home).is_ok() {
            if let Ok((ho, hw)) = ctx.fresh_binding(home) {
                if hw.pct <= threshold - margin {
                    if let Some(left) = cooldown_left {
                        return Proactive::None(format!("return home waits for cooldown ({} s)", left / 1000));
                    }
                    let home_label = ctx.account(home).map(|a| a.label.as_str()).unwrap_or("?");
                    return Proactive::Switch {
                        to: home.clone(),
                        reason: format!(
                            "home account {home_label} has room again: {:.0}% {} ({} old)",
                            hw.pct,
                            hw.key,
                            fmt_age(ho.age_ms(ctx.now) / 1000)
                        ),
                        return_home: true,
                    };
                }
            }
        }
    }

    if live_w.pct < threshold {
        return Proactive::None(format!(
            "{live_label} at {:.0}% {} ({} old), below {threshold:.0}%",
            live_w.pct,
            live_w.key,
            fmt_age(live_obs.age_ms(ctx.now) / 1000)
        ));
    }
    if let Some(left) = cooldown_left {
        return Proactive::None(format!(
            "{live_label} at {:.0}% {}; cooldown {} s after the last switch",
            live_w.pct,
            live_w.key,
            left / 1000
        ));
    }
    let picks = candidates(ctx, threshold - margin);
    match picks.first() {
        None => Proactive::None(format!(
            "{live_label} at {:.0}% {}, but no approved account has {:.0}% or less with a fresh observation; continuing",
            live_w.pct,
            live_w.key,
            threshold - margin
        )),
        Some(c) => {
            let to_label = ctx.account(&c.id).map(|a| a.label.as_str()).unwrap_or("?");
            Proactive::Switch {
                to: c.id.clone(),
                reason: format!(
                    "{live_label} at {:.0}% {} >= {threshold:.0}%; {to_label} at {:.0}% {} ({} old)",
                    live_w.pct,
                    live_w.key,
                    c.pct,
                    c.window,
                    fmt_age(c.age_s)
                ),
                return_home: false,
            }
        }
    }
}

/// Hard-limit confirmation (FR-15 step 1): the session's own rate limits first,
/// then a fresh engine observation of the live account.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitEvidence {
    pub confirmed: bool,
    pub detail: String,
}

pub fn confirm_limit(ctx: &Ctx, session_pcts: &[(String, f64)], simulated: bool) -> LimitEvidence {
    let hard = ctx.tuning.hard_limit_pct;
    if simulated {
        return LimitEvidence {
            confirmed: true,
            detail: "simulated usage limit".into(),
        };
    }
    if let Some((kind, pct)) = session_pcts
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    {
        return LimitEvidence {
            confirmed: *pct >= hard,
            detail: format!("{pct:.0}% {kind} as seen by the session"),
        };
    }
    let Some(live) = ctx.live else {
        return LimitEvidence {
            confirmed: false,
            detail: "no registered login is live".into(),
        };
    };
    match ctx.fresh_binding(live) {
        Ok((o, w)) => LimitEvidence {
            confirmed: w.pct >= hard,
            detail: format!("{:.0}% {} ({} old)", w.pct, w.key, fmt_age(o.age_ms(ctx.now) / 1000)),
        },
        Err(e) => LimitEvidence {
            confirmed: false,
            detail: format!("usage unknown: {}", e.explain()),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryTarget {
    /// An account with known remaining allowance.
    Capacity(Candidate),
    /// Every eligible account is exhausted: the one available soonest.
    Soonest {
        id: AccountId,
        available_at: i64,
    },
    None(String),
}

/// When an account becomes available: the latest reset among its exhausted windows.
/// `None` when an exhausted window has no known reset time.
pub fn available_at(o: &Observation, hard: f64, now: i64) -> Option<i64> {
    let mut latest = now;
    for w in o.windows.iter().filter(|w| w.pct >= hard) {
        latest = latest.max(w.resets_at?);
    }
    Some(latest)
}

pub fn recovery_target(ctx: &Ctx) -> RecoveryTarget {
    let hard = ctx.tuning.hard_limit_pct;
    let picks = candidates(ctx, hard - 0.0001);
    if let Some(c) = picks.into_iter().next() {
        return RecoveryTarget::Capacity(c);
    }
    // All exhausted: soonest available among permitted accounts with an observation
    // (age does not matter for a reset time already known).
    let mut best: Option<(AccountId, i64)> = None;
    let members = ctx.governing.first().map(|g| g.accounts.clone()).unwrap_or_default();
    for id in members.iter() {
        if ctx.live == Some(id) || ctx.permitted(id).is_err() {
            continue;
        }
        let Some(o) = ctx.observation(id) else { continue };
        if let Some(at) = available_at(o, hard, ctx.now) {
            if best.as_ref().is_none_or(|(_, b)| at < *b) {
                best = Some((id.clone(), at));
            }
        }
    }
    // The live account's own availability: switching only helps if another opens sooner.
    let live_at = ctx
        .live
        .and_then(|l| ctx.observation(l))
        .and_then(|o| available_at(o, hard, ctx.now));
    match best {
        Some((id, at)) if live_at.is_none_or(|l| at < l) => RecoveryTarget::Soonest { id, available_at: at },
        Some(_) => {
            RecoveryTarget::None("the current account becomes available soonest; Claude's own wait applies".into())
        }
        None => RecoveryTarget::None("no approved account has known capacity or a known reset time".into()),
    }
}

#[cfg(test)]
mod tests;
