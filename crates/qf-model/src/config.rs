//! User configuration: groups, project assignments, modes, tuning and consents.
//! Stored as `%LOCALAPPDATA%\Quayflow\config.json`. Only the engine writes it.

use crate::{Account, AccountId, AccountKind, Rank};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_GROUP: &str = "Default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Manual,
    #[default]
    Suggest,
    Automatic,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "manual" => Some(Mode::Manual),
            "suggest" => Some(Mode::Suggest),
            "automatic" | "auto" => Some(Mode::Automatic),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Manual => "manual",
            Mode::Suggest => "suggest",
            Mode::Automatic => "automatic",
        }
    }
}

/// Recorded acceptance of the one-time mixed-identity disclosure for a membership set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixedApproval {
    /// Sorted account ids the disclosure covered. A broader set needs a new acceptance.
    pub accounts: Vec<AccountId>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub accounts: Vec<AccountId>,
    /// Preferred account = home. At most one per group.
    #[serde(default)]
    pub preferred: Option<AccountId>,
    /// Optional ranks; missing means Normal. Preferred is implied for `preferred`.
    #[serde(default)]
    pub ranks: BTreeMap<AccountId, Rank>,
    #[serde(default)]
    pub mode: Mode,
    /// Automatic was explicitly approved for this group (opt-in).
    #[serde(default)]
    pub automatic_approved_at: Option<i64>,
    #[serde(default = "d_threshold")]
    pub threshold: f64,
    #[serde(default = "d_margin")]
    pub margin: f64,
    #[serde(default)]
    pub mixed_approval: Option<MixedApproval>,
}

fn d_threshold() -> f64 {
    85.0
}
fn d_margin() -> f64 {
    10.0
}

impl Group {
    pub fn new(name: &str) -> Self {
        Group {
            name: name.into(),
            accounts: vec![],
            preferred: None,
            ranks: BTreeMap::new(),
            mode: Mode::Suggest,
            automatic_approved_at: None,
            threshold: d_threshold(),
            margin: d_margin(),
            mixed_approval: None,
        }
    }

    pub fn rank_of(&self, id: &AccountId) -> Rank {
        if self.preferred.as_ref() == Some(id) {
            return Rank::Preferred;
        }
        self.ranks.get(id).copied().unwrap_or_default()
    }

    pub fn contains(&self, id: &AccountId) -> bool {
        self.accounts.contains(id)
    }

    /// Whether the members mix identities: organization with personal, or several organizations.
    pub fn is_mixed(&self, accounts: &[Account]) -> bool {
        mixed_identities(
            self.accounts
                .iter()
                .filter_map(|id| accounts.iter().find(|a| &a.id == id)),
        )
    }

    /// Whether the current membership is covered by a recorded disclosure acceptance.
    pub fn mixed_covered(&self) -> bool {
        match &self.mixed_approval {
            None => false,
            Some(a) => self.accounts.iter().all(|id| a.accounts.contains(id)),
        }
    }
}

pub fn mixed_identities<'a>(members: impl Iterator<Item = &'a Account>) -> bool {
    let members: Vec<&Account> = members.collect();
    let orgs: Vec<&str> = members
        .iter()
        .filter(|a| a.kind == AccountKind::Organization)
        .map(|a| a.org.as_str())
        .collect();
    let has_personal = members.iter().any(|a| a.kind == AccountKind::Personal);
    let mut distinct = orgs.clone();
    distinct.sort();
    distinct.dedup();
    (!orgs.is_empty() && has_personal) || distinct.len() > 1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAssignment {
    /// Absolute project folder; matches the folder and everything below it.
    pub path: String,
    pub group: String,
}

/// Tuning defaults from the design tokens. Every field has a default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tuning {
    pub cooldown_s: u64,
    pub pin_minutes: u64,
    pub pin_release_pct: f64,
    pub hard_limit_pct: f64,
    pub idle_wait_s: u64,
    pub lock_wait_s: u64,
    pub live_poll_s: u64,
    pub idle_poll_s: u64,
    pub live_fresh_s: u64,
    pub idle_fresh_s: u64,
    pub backoff_max_s: u64,
    pub hold_max_s: u64,
    pub log_retention_days: u64,
    pub log_max_mb: u64,
    /// A concurrent failure within this window retries on the account already switched to.
    pub concurrent_window_s: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            cooldown_s: 300,
            pin_minutes: 120,
            pin_release_pct: 98.0,
            hard_limit_pct: 99.0,
            idle_wait_s: 60,
            lock_wait_s: 9,
            live_poll_s: 60,
            idle_poll_s: 600,
            live_fresh_s: 300,
            idle_fresh_s: 3600,
            backoff_max_s: 900,
            hold_max_s: 900,
            log_retention_days: 7,
            log_max_mb: 20,
            concurrent_window_s: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Consents {
    /// The Mod may start the engine on demand.
    pub engine_start: Option<i64>,
    /// Engine starts at Windows sign-in (startup entry installed).
    pub signin_startup: Option<i64>,
    /// Quayflow may store reusable credential copies in its protected vault.
    pub credential_storage: Option<i64>,
    /// Terms and organization-rules disclaimer acknowledged.
    pub terms: Option<i64>,
    /// PowerShell prompt hook installed (same-terminal replacement).
    pub prompt_hook: Option<i64>,
    /// The manual live canary may run synthetic jobs.
    pub live_canary: Option<i64>,
}

/// D-ORG managed-login override (FR-06a). Read and displayed; this build contains
/// no code path that acts on it (see docs/tech-spec.md, decision B5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DOrgSetting {
    pub enabled: bool,
    /// Opaque hash of the organization named in the confirmation.
    pub confirmed_org: Option<String>,
    pub confirmed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "d_version")]
    pub version: u32,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default = "d_groups")]
    pub groups: Vec<Group>,
    #[serde(default = "d_default_group")]
    pub default_group: String,
    #[serde(default)]
    pub projects: Vec<ProjectAssignment>,
    #[serde(default)]
    pub tuning: Tuning,
    #[serde(default)]
    pub consents: Consents,
    /// Automation paused by the user (manual switches still work).
    #[serde(default)]
    pub automation_paused: bool,
    /// Unknown live logins whose notice the user acknowledged (B1: automation stays paused).
    #[serde(default)]
    pub ignored_unknown: Vec<String>,
    #[serde(default)]
    pub d_org: DOrgSetting,
    /// Onboarding completed (welcome and consents shown).
    #[serde(default)]
    pub onboarded_at: Option<i64>,
}

fn d_version() -> u32 {
    1
}
fn d_groups() -> Vec<Group> {
    vec![Group::new(DEFAULT_GROUP)]
}
fn d_default_group() -> String {
    DEFAULT_GROUP.into()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            accounts: vec![],
            groups: d_groups(),
            default_group: d_default_group(),
            projects: vec![],
            tuning: Tuning::default(),
            consents: Consents::default(),
            automation_paused: false,
            ignored_unknown: vec![],
            d_org: DOrgSetting::default(),
            onboarded_at: None,
        }
    }
}

impl Config {
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name.eq_ignore_ascii_case(name))
    }
    pub fn group_mut(&mut self, name: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.name.eq_ignore_ascii_case(name))
    }
    pub fn account(&self, id: &AccountId) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.id == id)
    }
    pub fn account_mut(&mut self, id: &AccountId) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| &a.id == id)
    }
    /// Find an account by id, id prefix, or label (case-insensitive).
    pub fn find_account(&self, key: &str) -> Option<&Account> {
        let k = key.trim();
        self.accounts
            .iter()
            .find(|a| a.id.0 == k || a.label.eq_ignore_ascii_case(k))
            .or_else(|| {
                let matches: Vec<&Account> = self
                    .accounts
                    .iter()
                    .filter(|a| k.len() >= 4 && a.id.0.starts_with(k))
                    .collect();
                if matches.len() == 1 {
                    Some(matches[0])
                } else {
                    None
                }
            })
    }
    pub fn label(&self, id: &AccountId) -> String {
        self.account(id)
            .map(|a| a.label.clone())
            .unwrap_or_else(|| format!("#{id}"))
    }

    /// FR-05: explicit trusted assignment (longest matching path) else the default group.
    pub fn resolve_group(&self, cwd: Option<&str>) -> &str {
        if let Some(cwd) = cwd {
            let c = normalize_path(cwd);
            let best = self
                .projects
                .iter()
                .filter(|p| {
                    let p = normalize_path(&p.path);
                    c == p || c.starts_with(&format!("{p}\\"))
                })
                .max_by_key(|p| p.path.len());
            if let Some(p) = best {
                if self.group(&p.group).is_some() {
                    return &p.group;
                }
            }
        }
        &self.default_group
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.groups.is_empty() {
            return Err("config needs at least one group".into());
        }
        if self.group(&self.default_group).is_none() {
            return Err(format!("default group \"{}\" does not exist", self.default_group));
        }
        let mut names: Vec<String> = self.groups.iter().map(|g| g.name.to_ascii_lowercase()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        if names.len() != before {
            return Err("group names must be unique (case-insensitive)".into());
        }
        for g in &self.groups {
            if g.name.trim().is_empty() || g.name.len() > 40 {
                return Err("group names must be 1-40 characters".into());
            }
            if !(g.threshold > 0.0 && g.threshold <= 100.0) {
                return Err(format!("group \"{}\": threshold must be in 1-100", g.name));
            }
            if !(g.margin >= 0.0 && g.margin < g.threshold) {
                return Err(format!(
                    "group \"{}\": margin must be 0 or more and below the threshold",
                    g.name
                ));
            }
            for id in &g.accounts {
                if self.account(id).is_none() {
                    return Err(format!("group \"{}\" lists unknown account #{id}", g.name));
                }
            }
            if let Some(p) = &g.preferred {
                if !g.accounts.contains(p) {
                    return Err(format!("group \"{}\": preferred account is not a member", g.name));
                }
            }
            if g.mode == Mode::Automatic && g.automatic_approved_at.is_none() {
                return Err(format!("group \"{}\": Automatic mode needs explicit approval", g.name));
            }
        }
        for p in &self.projects {
            if self.group(&p.group).is_none() {
                return Err(format!(
                    "project {} is assigned to unknown group \"{}\"",
                    p.path, p.group
                ));
            }
        }
        let mut ids: Vec<&AccountId> = self.accounts.iter().map(|a| &a.id).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        if ids.len() != n {
            return Err("duplicate account identity".into());
        }
        let mut labels: Vec<String> = self.accounts.iter().map(|a| a.label.to_ascii_lowercase()).collect();
        labels.sort();
        let n = labels.len();
        labels.dedup();
        if labels.len() != n {
            return Err("account labels must be unique".into());
        }
        let t = &self.tuning;
        if !(t.hard_limit_pct > 0.0 && t.hard_limit_pct <= 100.0)
            || !(t.pin_release_pct > 0.0 && t.pin_release_pct <= 100.0)
        {
            return Err("tuning percentages must be in 1-100".into());
        }
        if t.live_poll_s < 30 || t.idle_poll_s < 60 {
            return Err(
                "polling faster than 30 s (live) or 60 s (idle) is not allowed: the usage endpoint is rate-limited"
                    .into(),
            );
        }
        if t.lock_wait_s == 0 || t.lock_wait_s > 60 {
            return Err("lockWaitS must be 1-60".into());
        }
        Ok(())
    }
}

/// Lower-case, backslash, no trailing separator: for prefix comparison of Windows paths.
pub fn normalize_path(p: &str) -> String {
    let s = p.replace('/', "\\").to_ascii_lowercase();
    s.trim_end_matches('\\').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Health;

    fn acct(id: &str, kind: AccountKind, org: &str) -> Account {
        Account {
            id: AccountId(id.into()),
            label: id.into(),
            kind,
            org: org.into(),
            subscription: None,
            rate_limit_tier: None,
            registered_at: 0,
            health: Health::Ok,
            unavailable_models: vec![],
        }
    }

    #[test]
    fn default_config_is_valid() {
        let c = Config::default();
        c.validate().unwrap();
        assert_eq!(c.groups[0].name, "Default");
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
        // Missing fields take defaults.
        let min: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(min.tuning.cooldown_s, 300);
        assert_eq!(min.default_group, "Default");
    }

    #[test]
    fn rejects_invalid() {
        let mut c = Config::default();
        c.groups[0].threshold = 0.0;
        assert!(c.validate().unwrap_err().contains("threshold"));
        let mut c = Config::default();
        c.groups[0].mode = Mode::Automatic;
        assert!(c.validate().unwrap_err().contains("approval"));
        let mut c = Config::default();
        c.tuning.live_poll_s = 5;
        assert!(c.validate().is_err());
    }

    #[test]
    fn project_resolution_longest_match() {
        let mut c = Config::default();
        c.groups.push(Group::new("Work"));
        c.groups.push(Group::new("Inner"));
        c.projects.push(ProjectAssignment {
            path: "C:\\Work".into(),
            group: "Work".into(),
        });
        c.projects.push(ProjectAssignment {
            path: "C:\\Work\\inner".into(),
            group: "Inner".into(),
        });
        assert_eq!(c.resolve_group(Some("c:\\work\\a")), "Work");
        assert_eq!(c.resolve_group(Some("C:/Work/Inner/x")), "Inner");
        assert_eq!(c.resolve_group(Some("C:\\Workshop")), "Default");
        assert_eq!(c.resolve_group(None), "Default");
    }

    #[test]
    fn mixed_identity_detection() {
        let a = acct("a", AccountKind::Organization, "o1");
        let b = acct("b", AccountKind::Personal, "p1");
        let c = acct("c", AccountKind::Organization, "o2");
        let d = acct("d", AccountKind::Personal, "p2");
        assert!(mixed_identities([&a, &b].into_iter()));
        assert!(mixed_identities([&a, &c].into_iter()));
        assert!(!mixed_identities([&b, &d].into_iter()));
        assert!(!mixed_identities([&a].into_iter()));
    }

    #[test]
    fn mixed_approval_must_cover_membership() {
        let mut g = Group::new("G");
        g.accounts = vec![AccountId("a".into()), AccountId("b".into())];
        g.mixed_approval = Some(MixedApproval {
            accounts: vec![AccountId("a".into()), AccountId("b".into())],
            at: 1,
        });
        assert!(g.mixed_covered());
        g.accounts.push(AccountId("c".into()));
        assert!(!g.mixed_covered());
    }
}
