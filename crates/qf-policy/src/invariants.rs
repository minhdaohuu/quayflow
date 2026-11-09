//! Rule-based tests: thousands of generated situations, each decision checked against the
//! rules in CLAUDE.md ("Invariants"), not against expected values. A new decision branch is
//! covered here without anyone thinking of its corner cases. Deterministic: a failure prints
//! its seed; `QF_INVARIANT_SEED=<n>` replays that one case.

use super::*;
use qf_model::{AccountKind, Group, Health, Mode, Observation, Window};

const NOW: i64 = 1_000_000_000_000;
const MIN: i64 = 60_000;

/// xorshift64*: no dependency, same sequence on every platform.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pick<T: Clone>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize].clone()
    }
}

struct World {
    accounts: Vec<Account>,
    groups: Vec<Group>,
    governing: usize,
    obs: Vec<Observation>,
    tuning: Tuning,
    live: Option<AccountId>,
    model: Option<&'static str>,
    managed: ManagedLogin,
}

fn generate(seed: u64) -> World {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let n = 2 + r.below(4) as usize;
    let ids: Vec<AccountId> = (0..n).map(|i| AccountId(format!("acct{i}"))).collect();
    let accounts = ids
        .iter()
        .map(|id| Account {
            id: id.clone(),
            label: id.0.clone(),
            kind: if r.chance(50) {
                AccountKind::Personal
            } else {
                AccountKind::Organization
            },
            org: format!("org-{}", id.0),
            subscription: None,
            rate_limit_tier: None,
            registered_at: 0,
            health: r.pick(&[
                Health::Ok,
                Health::Ok,
                Health::Ok,
                Health::StaleAccess,
                Health::RefreshRejected,
                Health::VaultError,
            ]),
            unavailable_models: if r.chance(10) {
                vec!["claude-opus-5-5".into()]
            } else {
                vec![]
            },
            org_name: None,
        })
        .collect();
    let mut groups: Vec<Group> = vec![];
    for g in 0..1 + r.below(2) {
        let mut grp = Group::new(&format!("G{g}"));
        grp.accounts = ids.iter().filter(|_| r.chance(75)).cloned().collect();
        grp.mode = r.pick(&[Mode::Manual, Mode::Suggest, Mode::Automatic]);
        grp.threshold = r.pick(&[50.0, 80.0, 85.0, 95.0]);
        grp.margin = r.pick(&[0.0, 5.0, 10.0]);
        groups.push(grp);
    }
    let keys = ["5h", "7d", "7d:Opus"];
    let mut obs = vec![];
    for id in &ids {
        if !r.chance(85) {
            continue;
        }
        // Ages from fresh to far past every freshness limit.
        let ts = NOW - r.pick(&[0, 1, 4, 6, 30, 59, 61, 180]) * MIN;
        let mut windows = vec![];
        for k in keys {
            if !r.chance(70) {
                continue;
            }
            let pct = r.pick(&[0.0, 20.0, 60.0, 74.0, 76.0, 84.0, 86.0, 98.0, 99.0, 100.0]);
            let resets_at = match r.below(3) {
                0 => None,
                1 => Some(NOW - r.below(120) as i64 * MIN),
                _ => Some(NOW + (1 + r.below(600)) as i64 * MIN),
            };
            windows.push(Window {
                key: k.into(),
                pct,
                resets_at,
            });
        }
        obs.push(Observation {
            account: id.clone(),
            ts,
            windows,
            source: "engine".into(),
        });
    }
    let live = if r.chance(90) { Some(r.pick(&ids)) } else { None };
    World {
        accounts,
        governing: groups.len(),
        groups,
        obs,
        tuning: Tuning::default(),
        live,
        model: r.pick(&[None, Some("claude-opus-5-5"), Some("claude-sonnet-5-5")]),
        managed: ManagedLogin::default(),
    }
}

impl World {
    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            now: NOW,
            accounts: &self.accounts,
            governing: self.groups[..self.governing].iter().collect(),
            sessions: &[],
            all_groups: &self.groups,
            observations: &self.obs,
            live: self.live.as_ref(),
            managed: &self.managed,
            tuning: &self.tuning,
            pin: None,
            last_proactive_switch: None,
            model: self.model,
            busy_transition: false,
        }
    }
}

/// A destination must be allowed by every governing group and never be the live account.
fn allowed(ctx: &Ctx, id: &AccountId) -> Result<(), String> {
    if ctx.live == Some(id) {
        return Err("chose the live account".into());
    }
    ctx.permitted(id)
        .map(|_| ())
        .map_err(|e| format!("not permitted: {}", e.explain()))
}

/// "Stale is never zero": room may only be claimed from a fresh reading below the hard limit.
fn fresh_room(ctx: &Ctx, id: &AccountId) -> Result<(), String> {
    match ctx.fresh_binding(id) {
        Ok((_, w)) if w.pct < ctx.tuning.hard_limit_pct => Ok(()),
        Ok((_, w)) => Err(format!("claimed room at {:.0}%", w.pct)),
        Err(e) => Err(format!("claimed room without a fresh reading: {}", e.explain())),
    }
}

fn check(seed: u64) -> Result<(), String> {
    let w = generate(seed);
    let ctx = w.ctx();
    let hard = ctx.tuning.hard_limit_pct;

    // The strictest mode among the governing groups applies.
    let strictest = ctx.governing.iter().map(|g| g.mode).min().unwrap_or_default();
    if effective_mode(&ctx.governing) != strictest {
        return Err("effective mode is not the strictest".into());
    }

    if let Proactive::Switch { to, .. } = proactive(&ctx) {
        allowed(&ctx, &to).map_err(|e| format!("proactive: {e}"))?;
        fresh_room(&ctx, &to).map_err(|e| format!("proactive: {e}"))?;
        if ctx.live.is_none_or(|l| ctx.fresh_binding(l).is_err()) {
            return Err("proactive switch decided on unknown live usage".into());
        }
    }

    match recovery_target(&ctx) {
        RecoveryTarget::Capacity(c) => {
            allowed(&ctx, &c.id).map_err(|e| format!("recovery: {e}"))?;
            fresh_room(&ctx, &c.id).map_err(|e| format!("recovery: {e}"))?;
        }
        RecoveryTarget::Soonest { id, available_at } => {
            allowed(&ctx, &id).map_err(|e| format!("soonest: {e}"))?;
            // Availability is known only from an exhausted window with a known reset.
            let o = ctx.observation(&id).ok_or("soonest without an observation")?;
            let exhausted: Vec<&Window> = o
                .windows
                .iter()
                .filter(|x| x.pct >= hard && window_applies(x, ctx.model))
                .collect();
            if exhausted.is_empty() {
                return Err("soonest: no exhausted window, so availability is unknown, not now".into());
            }
            if exhausted.iter().any(|x| x.resets_at.is_none()) {
                return Err("soonest: an exhausted window has no reset time".into());
            }
            if available_at < ctx.now {
                return Err("soonest: available in the past".into());
            }
            // Never a fresh account with room: that would have been a Capacity target.
            if fresh_room(&ctx, &id).is_ok() {
                return Err("soonest chosen over a fresh account with room".into());
            }
        }
        RecoveryTarget::None(_) => {}
    }

    // Accounts a recovery polls on demand: exactly the allowed destinations with no reading or
    // a stale one, whenever no account with known room exists (Soonest and None included).
    let polled = poll_before_recovery(&ctx);
    let has_room = matches!(recovery_target(&ctx), RecoveryTarget::Capacity(_));
    if has_room && !polled.is_empty() {
        return Err("polled although an account with known room exists".into());
    }
    let unknown = |id: &AccountId| {
        ctx.observation(id)
            .is_none_or(|o| o.age_ms(ctx.now) / 1000 > ctx.fresh_limit_s(id))
    };
    for id in &polled {
        allowed(&ctx, id).map_err(|e| format!("poll_before_recovery: {e}"))?;
        if ctx.fresh_binding(id).is_ok() || !unknown(id) {
            return Err("poll_before_recovery listed an account with a fresh reading".into());
        }
    }
    if !has_room {
        for g in &ctx.governing {
            for id in &g.accounts {
                if allowed(&ctx, id).is_ok() && unknown(id) && !polled.contains(id) {
                    return Err(format!("{id:?} has unknown capacity but is not polled"));
                }
            }
        }
    }

    // The live account is read before confirming only when its usage is unknown and the
    // session sent no percentages; never with percentages, never for a fresh reading.
    if live_usage_unknown(&ctx, &[("five_hour".into(), 100.0)], false).is_some()
        || live_usage_unknown(&ctx, &[], true).is_some()
    {
        return Err("live usage read although the session sent percentages or simulated".into());
    }
    match (live_usage_unknown(&ctx, &[], false), ctx.live) {
        (Some(l), Some(live)) if &l == live && unknown(live) => {}
        (None, None) => {}
        (None, Some(live)) if !unknown(live) => {}
        (got, live) => return Err(format!("live_usage_unknown {got:?} for live {live:?}")),
    }

    for (id, r) in explain_all(&ctx) {
        if r.is_ok() {
            allowed(&ctx, &id).map_err(|e| format!("explain_all: {e}"))?;
            fresh_room(&ctx, &id).map_err(|e| format!("explain_all eligible: {e}"))?;
        }
    }
    Ok(())
}

#[test]
fn decisions_keep_the_invariants_in_generated_situations() {
    let seeds: Vec<u64> = match std::env::var("QF_INVARIANT_SEED") {
        Ok(s) => vec![s.parse().expect("QF_INVARIANT_SEED is a number")],
        Err(_) => (1..=20_000).collect(),
    };
    for seed in seeds {
        if let Err(e) = check(seed) {
            panic!("seed {seed}: {e}\nreplay: QF_INVARIANT_SEED={seed} cargo test -p qf-policy invariants");
        }
    }
}

#[test]
fn the_generator_reaches_every_decision() {
    // Guard against a generator that never produces the interesting cases.
    let (mut switch, mut capacity, mut soonest, mut polled) = (0, 0, 0, 0);
    for seed in 1..=20_000 {
        let w = generate(seed);
        let ctx = w.ctx();
        if !poll_before_recovery(&ctx).is_empty() {
            polled += 1;
        }
        if matches!(proactive(&ctx), Proactive::Switch { .. }) {
            switch += 1;
        }
        match recovery_target(&ctx) {
            RecoveryTarget::Capacity(_) => capacity += 1,
            RecoveryTarget::Soonest { .. } => soonest += 1,
            RecoveryTarget::None(_) => {}
        }
    }
    assert!(
        switch > 100 && capacity > 100 && soonest > 100 && polled > 100,
        "{switch} {capacity} {soonest} {polled}"
    );
}
