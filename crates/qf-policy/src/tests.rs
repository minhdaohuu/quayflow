use super::*;
use qf_model::{AccountKind, Group, Health, Mode, Observation, Window};

const MIN: i64 = 60_000;
const NOW: i64 = 1_000_000_000_000;

fn acct(id: &str, kind: AccountKind) -> Account {
    Account {
        id: AccountId(id.into()),
        label: id.into(),
        kind,
        org: format!("org-{id}"),
        subscription: None,
        rate_limit_tier: None,
        registered_at: 0,
        health: Health::Ok,
        unavailable_models: vec![],
    }
}

fn obs(id: &str, age_min: i64, windows: &[(&str, f64, Option<i64>)]) -> Observation {
    Observation {
        account: AccountId(id.into()),
        ts: NOW - age_min * MIN,
        windows: windows
            .iter()
            .map(|(k, p, r)| Window {
                key: (*k).into(),
                pct: *p,
                resets_at: *r,
            })
            .collect(),
        source: "engine".into(),
    }
}

fn group(name: &str, members: &[&str]) -> Group {
    let mut g = Group::new(name);
    g.accounts = members.iter().map(|m| AccountId((*m).into())).collect();
    g
}

struct World {
    accounts: Vec<Account>,
    groups: Vec<Group>,
    obs: Vec<Observation>,
    sessions: Vec<SessionInfo>,
    managed: ManagedLogin,
    tuning: Tuning,
    live: AccountId,
    pin: Option<Pin>,
    last: Option<i64>,
}

impl World {
    fn new() -> Self {
        World {
            accounts: vec![
                acct("a", AccountKind::Organization),
                acct("b", AccountKind::Personal),
                acct("c", AccountKind::Personal),
            ],
            groups: vec![group("Default", &["a", "b", "c"])],
            obs: vec![],
            sessions: vec![],
            managed: ManagedLogin::default(),
            tuning: Tuning::default(),
            live: AccountId("a".into()),
            pin: None,
            last: None,
        }
    }
    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            now: NOW,
            accounts: &self.accounts,
            governing: vec![&self.groups[0]],
            sessions: &self.sessions,
            all_groups: &self.groups,
            observations: &self.obs,
            live: Some(&self.live),
            managed: &self.managed,
            tuning: &self.tuning,
            pin: self.pin.as_ref(),
            last_proactive_switch: self.last,
            model: None,
            busy_transition: false,
        }
    }
}

fn switch_to(p: &Proactive) -> Option<&str> {
    match p {
        Proactive::Switch { to, .. } => Some(to.as_str()),
        _ => None,
    }
}

#[test]
fn below_threshold_does_nothing() {
    let mut w = World::new();
    w.obs = vec![obs("a", 0, &[("5h", 50.0, None)]), obs("b", 5, &[("5h", 0.0, None)])];
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
}

#[test]
fn threshold_switches_to_lowest_fresh_with_margin() {
    let mut w = World::new();
    w.obs = vec![
        obs("a", 0, &[("5h", 86.0, None)]),
        obs("b", 5, &[("5h", 70.0, None)]),
        obs("c", 5, &[("5h", 74.0, None), ("7d", 80.0, None)]),
    ];
    // margin 10 => candidate must be <= 75: b (70) qualifies, c binding is 7d 80 -> excluded
    assert_eq!(switch_to(&proactive(&w.ctx())), Some("b"));
}

#[test]
fn stale_is_never_zero() {
    let mut w = World::new();
    w.obs = vec![obs("a", 0, &[("5h", 90.0, None)]), obs("b", 61, &[("5h", 0.0, None)])];
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
    // Stale live observation: no decision at all.
    w.obs = vec![obs("a", 6, &[("5h", 99.0, None)]), obs("b", 1, &[("5h", 0.0, None)])];
    match proactive(&w.ctx()) {
        Proactive::None(why) => assert!(why.contains("not deciding"), "{why}"),
        p => panic!("{p:?}"),
    }
}

#[test]
fn cooldown_blocks_proactive() {
    let mut w = World::new();
    w.obs = vec![obs("a", 0, &[("5h", 90.0, None)]), obs("b", 1, &[("5h", 0.0, None)])];
    w.last = Some(NOW - 2 * MIN);
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
    w.last = Some(NOW - 6 * MIN);
    assert_eq!(switch_to(&proactive(&w.ctx())), Some("b"));
}

#[test]
fn pin_holds_until_release_level() {
    let mut w = World::new();
    w.pin = Some(Pin {
        account: AccountId("a".into()),
        until: NOW + 60 * MIN,
        at: NOW,
    });
    w.obs = vec![obs("a", 0, &[("5h", 90.0, None)]), obs("b", 1, &[("5h", 0.0, None)])];
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
    w.obs[0] = obs("a", 0, &[("5h", 98.0, None)]);
    assert_eq!(switch_to(&proactive(&w.ctx())), Some("b"));
    // Expired pin no longer holds.
    w.obs[0] = obs("a", 0, &[("5h", 90.0, None)]);
    w.pin = Some(Pin {
        account: AccountId("a".into()),
        until: NOW - 1,
        at: NOW - 200 * MIN,
    });
    assert_eq!(switch_to(&proactive(&w.ctx())), Some("b"));
}

#[test]
fn pin_until_is_earlier_of_period_and_reset() {
    let t = Tuning::default();
    assert_eq!(pin_until(NOW, &t, None), NOW + 120 * MIN);
    assert_eq!(pin_until(NOW, &t, Some(NOW + 30 * MIN)), NOW + 30 * MIN);
    assert_eq!(pin_until(NOW, &t, Some(NOW - 1)), NOW + 120 * MIN);
}

#[test]
fn return_home_when_preferred_has_room() {
    let mut w = World::new();
    w.groups[0].preferred = Some(AccountId("a".into()));
    w.live = AccountId("b".into());
    w.obs = vec![obs("a", 10, &[("5h", 20.0, None)]), obs("b", 0, &[("5h", 30.0, None)])];
    let p = proactive(&w.ctx());
    assert!(matches!(p, Proactive::Switch { return_home: true, .. }), "{p:?}");
    // Home not yet reset (above threshold - margin)
    w.obs[0] = obs("a", 10, &[("5h", 80.0, None)]);
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
    // Pinned on b: no return home.
    w.obs[0] = obs("a", 10, &[("5h", 20.0, None)]);
    w.pin = Some(Pin {
        account: AccountId("b".into()),
        until: NOW + MIN,
        at: NOW,
    });
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
}

#[test]
fn ranks_break_close_choices_only() {
    let c = |id: &str, pct: f64, rank: Rank| Candidate {
        id: AccountId(id.into()),
        pct,
        window: "5h".into(),
        rank,
        age_s: 0,
    };
    let r = rank_candidates(vec![
        c("x", 10.0, Rank::Normal),
        c("y", 15.0, Rank::Preferred),
        c("z", 5.0, Rank::Backup),
    ]);
    assert_eq!(r[0].id.as_str(), "y");
    // A nearly exhausted preference does not defeat a clearly suitable alternative.
    let r = rank_candidates(vec![c("x", 10.0, Rank::Normal), c("y", 70.0, Rank::Preferred)]);
    assert_eq!(r[0].id.as_str(), "x");
}

#[test]
fn managed_login_and_health_exclude() {
    let mut w = World::new();
    w.obs = vec![
        obs("a", 0, &[("5h", 90.0, None)]),
        obs("b", 1, &[("5h", 0.0, None)]),
        obs("c", 1, &[("5h", 5.0, None)]),
    ];
    w.managed = ManagedLogin {
        force_org: vec!["org-a".into(), "org-c".into()],
        ..Default::default()
    };
    assert_eq!(switch_to(&proactive(&w.ctx())), Some("c"));
    w.accounts[2].health = Health::RefreshRejected;
    assert_eq!(switch_to(&proactive(&w.ctx())), None);
}

#[test]
fn plan_incompatibility_excludes() {
    let mut w = World::new();
    w.obs = vec![obs("a", 0, &[("5h", 90.0, None)]), obs("b", 1, &[("5h", 0.0, None)])];
    w.accounts[1].unavailable_models = vec!["claude-opus-5-5".into()];
    let mut ctx = w.ctx();
    ctx.model = Some("claude-opus-5-5");
    assert_eq!(switch_to(&proactive(&ctx)), None);
    ctx.model = Some("claude-sonnet-5-5");
    assert_eq!(switch_to(&proactive(&ctx)), Some("b"));
}

#[test]
fn group_conflict_names_blocker() {
    let mut w = World::new();
    w.groups.push(group("Work", &["a", "c"]));
    w.sessions = vec![SessionInfo {
        pid: 42,
        session_id: "s".into(),
        env: qf_model::EnvId::default_env(),
        name: Some("work-session".into()),
        kind: None,
        status: None,
        cwd: None,
        version: None,
        group: Some("Work".into()),
    }];
    let ctx = Ctx {
        governing: vec![&w.groups[0], &w.groups[1]],
        ..w.ctx()
    };
    match ctx.permitted(&AccountId("b".into())) {
        Err(Exclusion::GroupConflict { session, group }) => {
            assert_eq!(session, "work-session");
            assert_eq!(group, "Work");
        }
        r => panic!("{r:?}"),
    }
    assert_eq!(ctx.blocker(&AccountId("b".into())).unwrap().pid, 42);
    assert!(ctx.permitted(&AccountId("c".into())).is_ok());
}

#[test]
fn effective_mode_is_most_restrictive() {
    let mut a = Group::new("A");
    a.mode = Mode::Automatic;
    let b = Group::new("B");
    assert_eq!(effective_mode(&[&a, &b]), Mode::Suggest);
    assert_eq!(effective_mode(&[&a]), Mode::Automatic);
}

#[test]
fn limit_confirmation_prefers_session_view() {
    let mut w = World::new();
    w.obs = vec![obs("a", 0, &[("5h", 50.0, None)])];
    let ctx = w.ctx();
    assert!(confirm_limit(&ctx, &[("five_hour".into(), 100.0)], false).confirmed);
    assert!(!confirm_limit(&ctx, &[("five_hour".into(), 40.0)], false).confirmed);
    // No session view: engine observation (50%) does not confirm.
    assert!(!confirm_limit(&ctx, &[], false).confirmed);
    w.obs = vec![obs("a", 1, &[("7d", 99.5, None)])];
    assert!(confirm_limit(&w.ctx(), &[], false).confirmed);
    // Stale engine observation never confirms.
    w.obs = vec![obs("a", 30, &[("7d", 100.0, None)])];
    assert!(!confirm_limit(&w.ctx(), &[], false).confirmed);
}

#[test]
fn recovery_prefers_capacity_then_soonest() {
    let mut w = World::new();
    w.obs = vec![
        obs("a", 0, &[("5h", 100.0, Some(NOW + 300 * MIN))]),
        obs("b", 5, &[("5h", 60.0, None)]),
        obs("c", 5, &[("5h", 30.0, None)]),
    ];
    match recovery_target(&w.ctx()) {
        RecoveryTarget::Capacity(c) => assert_eq!(c.id.as_str(), "c"),
        r => panic!("{r:?}"),
    }
    // All exhausted: soonest is the account whose exhausted windows all reset first.
    w.obs = vec![
        obs("a", 0, &[("5h", 100.0, Some(NOW + 300 * MIN))]),
        obs(
            "b",
            5,
            &[
                ("5h", 100.0, Some(NOW + 60 * MIN)),
                ("7d", 100.0, Some(NOW + 600 * MIN)),
            ],
        ),
        obs(
            "c",
            5,
            &[("5h", 100.0, Some(NOW + 120 * MIN)), ("7d", 50.0, Some(NOW + 10 * MIN))],
        ),
    ];
    match recovery_target(&w.ctx()) {
        RecoveryTarget::Soonest { id, available_at } => {
            assert_eq!(id.as_str(), "c");
            assert_eq!(available_at, NOW + 120 * MIN);
        }
        r => panic!("{r:?}"),
    }
    // The live account resets first: no switch.
    w.obs[0] = obs("a", 0, &[("5h", 100.0, Some(NOW + 5 * MIN))]);
    assert!(matches!(recovery_target(&w.ctx()), RecoveryTarget::None(_)));
    // Unknown reset times are not ranked.
    w.obs = vec![obs("a", 0, &[("5h", 100.0, None)]), obs("b", 5, &[("5h", 100.0, None)])];
    assert!(matches!(recovery_target(&w.ctx()), RecoveryTarget::None(_)));
}

#[test]
fn per_model_window_counts_for_that_model_only() {
    let mut w = World::new();
    w.obs = vec![
        obs("a", 0, &[("5h", 90.0, None)]),
        obs("b", 1, &[("5h", 10.0, None), ("7d:Opus", 99.0, None)]),
    ];
    let mut ctx = w.ctx();
    ctx.model = Some("claude-opus-5-5");
    assert_eq!(switch_to(&proactive(&ctx)), None);
    ctx.model = Some("claude-sonnet-5-5");
    assert_eq!(switch_to(&proactive(&ctx)), Some("b"));
}
