//! Snapshots for the Mod, the CLI and support reports.

use crate::core::Engine;
use qf_model::api::*;
use qf_model::time::now_ms;
use qf_model::*;

impl Engine {
    pub fn state_view(&self, session_id: Option<&str>, cwd: Option<&str>) -> StateView {
        let lanes = self.lane_views();
        let inner = self.lock();
        let now = now_ms();
        let cfg = &inner.config;
        let mut envs = vec![];
        for (id, st) in &inner.envs {
            let governing: Vec<&Group> = Engine::governing(&inner, id);
            let mut notices = vec![];
            let mut push = |level: &str, kind: &str, text: String, persistent: bool| {
                notices.push(Notice {
                    id: format!("{kind}:{id}"),
                    level: level.into(),
                    kind: kind.into(),
                    text,
                    persistent,
                })
            };
            if cfg.automation_paused {
                push("warn", "paused", "Automation is paused (/quay resume).".into(), true);
            }
            if let Some(u) = &st.unknown {
                push(
                    "warn",
                    "unknown_login",
                    format!(
                        "The live login (#{u}) is not registered. Automatic switching and recovery are paused here. Register it (/quay register) or ignore.{}",
                        if cfg.ignored_unknown.contains(u) { " Notice acknowledged." } else { "" }
                    ),
                    true,
                );
            }
            if st.other_switcher {
                push("warn", "other_switcher", "The login changed repeatedly without Quayflow: another tool may be switching. Automation is paused.".into(), true);
            }
            if !st.in_place.allowed {
                push(
                    "warn",
                    "incompatible_version",
                    format!("In-place switching is off: {}.", st.in_place.reason),
                    true,
                );
            }
            if st.managed.is_enforced() {
                push(
                    "info",
                    "managed_login",
                    format!(
                        "Your organization enforces its login here ({}). Accounts that don't match are not offered; the administrator controls this.",
                        st.managed.sources.join(", ")
                    ),
                    true,
                );
            }
            if cfg.d_org.enabled {
                push(
                    "warn",
                    "d_org_enabled",
                    "Managed-login override setting is ON (recorded only; this build still respects enforcement)."
                        .into(),
                    true,
                );
            }
            if st.adoption.iter().any(|a| a.state == AdoptionState::Mismatch) {
                push(
                    "error",
                    "adoption_mismatch",
                    "A session's next request did not use the switched login: in-place switching is off for that Claude version. Review the full-history fallback in /quay.".into(),
                    true,
                );
            }
            for a in &cfg.accounts {
                match a.health {
                    Health::RefreshRejected => push(
                        "warn",
                        "refresh_rejected",
                        format!("{} needs a new sign-in; it is set aside.", a.label),
                        true,
                    ),
                    Health::VaultError => push(
                        "error",
                        "vault_error",
                        format!("{}: its protected copy cannot be read.", a.label),
                        true,
                    ),
                    _ => {}
                }
            }
            if let Some(s) = &st.suggestion {
                push(
                    "info",
                    "suggestion",
                    format!("Switch to {}? {} (/quay accept)", cfg.label(&s.to), s.reason),
                    false,
                );
            }
            // Kind and plan of the live login, for review before registering it (no token is exposed).
            let live_kind: Option<(AccountKind, Option<String>)> = match (&st.live, &st.unknown) {
                (Some(l), None) => cfg.account(l).map(|a| (a.kind, a.subscription.clone())),
                (Some(_), Some(_)) => qf_adapter::read_login(&st.env)
                    .ok()
                    .flatten()
                    .map(|l| (l.kind(), l.subscription().map(String::from))),
                _ => None,
            };
            let managed_org_names: Vec<String> = st
                .managed
                .force_org
                .iter()
                .filter_map(|h| {
                    cfg.accounts
                        .iter()
                        .find(|a| &a.org == h)
                        .and_then(|a| a.org_name.clone())
                })
                .collect();
            envs.push(EnvView {
                id: id.clone(),
                config_dir: st.env.config_dir.display().to_string(),
                live: LiveLogin {
                    account: st.live.clone(),
                    known: st.live.is_some() && st.unknown.is_none(),
                    label: st.live.as_ref().filter(|_| st.unknown.is_none()).map(|l| cfg.label(l)),
                    signed_out: st.signed_out,
                    kind: live_kind.as_ref().map(|x| x.0),
                    subscription: live_kind.as_ref().and_then(|x| x.1.clone()),
                },
                mode: qf_policy::effective_mode(&governing),
                governing_groups: governing.iter().map(|g| g.name.clone()).collect(),
                in_place: st.in_place.clone(),
                managed: st.managed.clone(),
                managed_org_names,
                transition: st.transition.clone(),
                last_switch: st.last_switch.clone(),
                pin: st.pin.clone().filter(|p| p.until > now),
                suggestion: st.suggestion.clone(),
                adoption: st.adoption.clone(),
                sessions: st.sessions.clone(),
                decision: st.decision.clone(),
                automation_paused: st.pause_reason(cfg).is_some(),
                paused_reason: st.pause_reason(cfg),
                paused_by: st.pause_kind(cfg).map(String::from),
                notices,
            });
        }
        let session = session_id.map(|sid| {
            let env = self.env_of_session(&inner, sid);
            let st = inner.envs.get(&env);
            let cwd = st
                .and_then(|s| s.sessions.iter().find(|x| x.session_id == sid))
                .and_then(|s| s.cwd.clone())
                .or_else(|| cwd.map(String::from));
            SessionView {
                session_id: sid.into(),
                group: cfg.resolve_group(cwd.as_deref()).to_string(),
                group_source: if cfg.projects.iter().any(|p| {
                    let pp = normalize_path(&p.path);
                    cwd.as_deref()
                        .map(normalize_path)
                        .is_some_and(|c| c == pp || c.starts_with(&format!("{pp}\\")))
                }) {
                    "assigned".into()
                } else {
                    "default".into()
                },
                verified_account: inner.verified.get(sid).map(|v| v.0.clone()),
                verified_at: inner.verified.get(sid).map(|v| v.1),
                adoption: st.and_then(|s| s.adoption.iter().rev().find(|a| a.session_id == sid).cloned()),
                env,
            }
        });
        let default_env = self.default_env.id.clone();
        let accounts = cfg
            .accounts
            .iter()
            .map(|a| {
                let live_in: Vec<EnvId> = inner
                    .envs
                    .iter()
                    .filter(|(_, s)| s.live.as_ref() == Some(&a.id))
                    .map(|(i, _)| i.clone())
                    .collect();
                let fresh_limit = if live_in.is_empty() {
                    cfg.tuning.idle_fresh_s
                } else {
                    cfg.tuning.live_fresh_s
                } as i64;
                let usage = inner
                    .observations
                    .iter()
                    .find(|o| o.account == a.id)
                    .map(|o| UsageView {
                        binding: o.binding().cloned(),
                        windows: o.windows.clone(),
                        age_s: o.age_ms(now) / 1000,
                        fresh: o.age_ms(now) / 1000 <= fresh_limit,
                        source: o.source.clone(),
                    });
                let env_for = live_in.first().cloned().unwrap_or_else(|| default_env.clone());
                let eligibility = Engine::ctx(&inner, &env_for, None).map(|ctx| {
                    let all = qf_policy::explain_all(&ctx);
                    match all.into_iter().find(|(id, _)| id == &a.id).map(|(_, r)| r) {
                        Some(Ok(why)) => Eligibility {
                            eligible: true,
                            reason: why,
                        },
                        Some(Err(e)) => Eligibility {
                            eligible: false,
                            reason: e.explain(),
                        },
                        None => Eligibility {
                            eligible: false,
                            reason: "unknown".into(),
                        },
                    }
                });
                AccountView {
                    id: a.id.clone(),
                    label: a.label.clone(),
                    kind: a.kind,
                    subscription: a.subscription.clone(),
                    health: a.health,
                    groups: cfg
                        .groups
                        .iter()
                        .filter(|g| g.contains(&a.id))
                        .map(|g| g.name.clone())
                        .collect(),
                    live_in,
                    usage,
                    usage_note: inner.schedule.note.get(a.id.as_str()).cloned(),
                    eligibility,
                    unavailable_models: a.unavailable_models.clone(),
                    rank: cfg.groups.iter().find(|g| g.contains(&a.id)).map(|g| g.rank_of(&a.id)),
                    org: a.org.clone(),
                    org_name: a.org_name.clone(),
                }
            })
            .collect();
        let groups = cfg
            .groups
            .iter()
            .map(|g| GroupView {
                group: g.clone(),
                is_default: g.name == cfg.default_group,
                mixed: g.is_mixed(&cfg.accounts),
                mixed_covered: g.mixed_covered(),
            })
            .collect();
        StateView {
            engine: EngineInfo {
                version: ENGINE_VERSION.into(),
                protocol: PROTOCOL,
                pid: std::process::id(),
                started_at: self.started_at,
                data_dir: self.dd.root.display().to_string(),
                integrity: "not code-signed; verify with the release's SHA256SUMS (and gh attestation verify when the release is attested)"
                    .into(),
                exe_sha256: self.exe_sha256.clone(),
                simulation: qf_model::sim::api_base(),
            },
            onboarded: cfg.onboarded_at.is_some(),
            consents: cfg.consents.clone(),
            session,
            envs,
            accounts,
            groups,
            default_group: cfg.default_group.clone(),
            projects: cfg.projects.clone(),
            tuning: cfg.tuning.clone(),
            compat: inner.compat.values().cloned().collect(),
            d_org: cfg.d_org.clone(),
            lanes,
            activity: inner.activity.iter().rev().take(100).cloned().collect(),
            seq: self.seq(),
        }
    }
}
