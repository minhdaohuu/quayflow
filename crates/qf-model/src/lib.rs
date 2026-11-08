//! Shared Quayflow types. Pure data: no I/O.
//!
//! Identifiers are opaque: an account is known by the first 12 hex characters
//! of the SHA-256 of its provider account UUID (the same hash the Mod computes
//! from the profile endpoint), never by email or token.

pub mod api;
pub mod config;
pub mod sim;
pub mod time;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub use config::*;

/// Current protocol of the Mod/engine API. The engine also accepts `PROTOCOL - 1`.
pub const PROTOCOL: u32 = 1;
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Opaque identity hash: first 12 hex characters of SHA-256 of the value.
pub fn identity_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..6])
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn from_uuid(uuid: &str) -> Self {
        AccountId(identity_hash(uuid))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Organization,
    Personal,
}

/// Credential health as far as Quayflow can tell without refreshing tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    #[default]
    Ok,
    /// Access token expired while idle; refreshes when the account is next live.
    StaleAccess,
    /// The refresh token was rejected: set aside until the user signs in again.
    RefreshRejected,
    /// The vault entry cannot be decrypted or is missing.
    VaultError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Rank {
    Preferred,
    #[default]
    Normal,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: AccountId,
    /// Friendly label chosen by the user. Shown in the UI; never written to logs.
    pub label: String,
    pub kind: AccountKind,
    /// Opaque hash of the organization UUID (personal accounts have a personal organization).
    pub org: String,
    /// Provider subscription type as reported (for example "team", "max", "pro").
    #[serde(default)]
    pub subscription: Option<String>,
    #[serde(default)]
    pub rate_limit_tier: Option<String>,
    pub registered_at: i64,
    #[serde(default)]
    pub health: Health,
    /// Models learned to be unavailable for this account (`model_not_found` after a switch).
    #[serde(default)]
    pub unavailable_models: Vec<String>,
    /// Organization display name from the local login record. Shown in the UI only (never logged).
    #[serde(default)]
    pub org_name: Option<String>,
}

/// One usage window of one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// "5h", "7d", or "7d:<model>" for per-model weekly windows.
    pub key: String,
    pub pct: f64,
    /// Reset time, epoch milliseconds.
    #[serde(default)]
    pub resets_at: Option<i64>,
}

/// A usage observation with its timestamp. Missing or stale never means zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub account: AccountId,
    pub ts: i64,
    pub windows: Vec<Window>,
    /// "engine" (usage endpoint) or "session" (rate limits seen by a client).
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "engine".into()
}

impl Observation {
    /// The most constrained relevant window.
    pub fn binding(&self) -> Option<&Window> {
        self.windows
            .iter()
            .filter(|w| w.pct.is_finite())
            .max_by(|a, b| a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Windows relevant to a model: 5h, 7d and the model's own weekly window.
    pub fn binding_for(&self, model: Option<&str>) -> Option<&Window> {
        self.windows
            .iter()
            .filter(|w| match (w.key.strip_prefix("7d:"), model) {
                (Some(m), Some(model)) => model_matches(m, model),
                (Some(_), None) => true,
                (None, _) => true,
            })
            .max_by(|a, b| a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal))
    }

    pub fn age_ms(&self, now: i64) -> i64 {
        (now - self.ts).max(0)
    }
}

/// A usage window's key as people read it: "5h", "7d", "7d Opus" (the key "7d:Opus").
pub fn window_name(key: &str) -> String {
    key.replacen(':', " ", 1)
}

/// A per-model window label ("Opus", "Sonnet 4.6") matches a model id ("claude-opus-5-5")
/// when the window's family word appears in the id.
pub fn model_matches(window_model: &str, model_id: &str) -> bool {
    let family = window_model
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    !family.is_empty() && model_id.to_ascii_lowercase().contains(&family)
}

/// A credential environment: the default `~/.claude` or a lane.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnvId(pub String);

impl EnvId {
    pub fn default_env() -> Self {
        EnvId("default".into())
    }
    pub fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

impl fmt::Display for EnvId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A running Claude client from the session registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub pid: u32,
    pub session_id: String,
    pub env: EnvId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// Group the session's project resolves to (filled by the engine).
    #[serde(default)]
    pub group: Option<String>,
}

impl SessionInfo {
    pub fn is_busy(&self) -> bool {
        self.status.as_deref() == Some("busy")
    }
    /// Short display name without the full path.
    pub fn display(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        let project = self
            .cwd
            .as_deref()
            .and_then(|c| c.rsplit(['\\', '/']).find(|s| !s.is_empty()))
            .unwrap_or("?");
        format!("{project} (pid {})", self.pid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchHow {
    Manual,
    Accepted,
    Automatic,
    ReturnHome,
    Recovery,
    /// Switch back after `model_not_found` on the destination (counts as the incident's recovery).
    ModelFallback,
    Canary,
}

impl SwitchHow {
    pub fn pins(self) -> bool {
        matches!(self, SwitchHow::Manual)
    }
    pub fn is_proactive(self) -> bool {
        matches!(self, SwitchHow::Automatic | SwitchHow::ReturnHome | SwitchHow::Accepted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionState {
    Requested,
    Preparing,
    IdleWait,
    Applying,
    VerifyingAdoption,
    Completed,
    Cancelled,
    /// Ended before the write; the login is unchanged.
    Failed,
    /// Ended after the write without certainty; reconciled or shown as failure.
    Uncertain,
}

impl TransitionState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TransitionState::Completed
                | TransitionState::Cancelled
                | TransitionState::Failed
                | TransitionState::Uncertain
        )
    }
    pub fn written(self) -> bool {
        matches!(
            self,
            TransitionState::VerifyingAdoption | TransitionState::Completed | TransitionState::Uncertain
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub id: String,
    pub env: EnvId,
    #[serde(default)]
    pub from: Option<AccountId>,
    pub to: AccountId,
    pub how: SwitchHow,
    /// Sanitized reason (labels allowed in the UI state, not in logs).
    pub reason: String,
    pub state: TransitionState,
    pub requested_at: i64,
    #[serde(default)]
    pub written_at: Option<i64>,
    #[serde(default)]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub locked_ms: Option<u64>,
    #[serde(default)]
    pub detail: Option<String>,
    /// Machine code when the transition failed: "lock_held", "vault", "pre_write", "post_write", "cancelled".
    #[serde(default)]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionState {
    /// The client has not made a request since the write.
    PendingNextRequest,
    Adopted,
    Mismatch,
}

/// Per-client adoption: stored destination versus the identity its last request used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientAdoption {
    pub session_id: String,
    pub pid: u32,
    pub transition: String,
    pub destination: AccountId,
    #[serde(default)]
    pub last_request_account: Option<AccountId>,
    pub state: AdoptionState,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub account: AccountId,
    pub until: i64,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub ts: i64,
    pub env: EnvId,
    #[serde(default)]
    pub from: Option<AccountId>,
    pub to: AccountId,
    pub how: SwitchHow,
    pub reason: String,
}

/// Static and live version-check result for one Claude version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatRecord {
    pub version: String,
    pub checked_at: i64,
    pub static_pass: bool,
    #[serde(default)]
    pub static_detail: Vec<String>,
    /// `Some(false)` after a failed adoption verification on this version.
    #[serde(default)]
    pub live_pass: Option<bool>,
    #[serde(default)]
    pub live_detail: Option<String>,
}

impl CompatRecord {
    pub fn in_place_allowed(&self) -> bool {
        self.static_pass && self.live_pass != Some(false)
    }
}

/// Managed login enforcement for one environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManagedLogin {
    /// Opaque hash of the enforced organization UUID(s).
    #[serde(default)]
    pub force_org: Vec<String>,
    #[serde(default)]
    pub force_method: Option<String>,
    /// Where it came from: "registry:HKLM", "file:managed-settings.json", "server", ...
    #[serde(default)]
    pub sources: Vec<String>,
}

impl ManagedLogin {
    pub fn is_enforced(&self) -> bool {
        !self.force_org.is_empty() || self.force_method.is_some()
    }
    /// Whether an account matches the enforcement (subscription logins are "claudeai").
    pub fn permits(&self, account: &Account) -> bool {
        let org_ok = self.force_org.is_empty() || self.force_org.iter().any(|o| o == &account.org);
        let method_ok = match self.force_method.as_deref() {
            None => true,
            Some("claudeai") => true,
            Some(_) => false,
        };
        org_ok && method_ok
    }
}

/// Version comparison for dotted numeric versions ("2.1.316"); non-numeric parts compare as 0.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).map(|p| p.parse().unwrap_or(0)).collect() };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_poc() {
        // 12 hex characters, deterministic.
        let h = identity_hash("00000000-0000-0000-0000-000000000000");
        assert_eq!(h.len(), 12);
        assert_eq!(h, identity_hash("00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn binding_is_fullest_window() {
        let o = Observation {
            account: AccountId("a".into()),
            ts: 0,
            windows: vec![
                Window {
                    key: "5h".into(),
                    pct: 40.0,
                    resets_at: None,
                },
                Window {
                    key: "7d".into(),
                    pct: 72.0,
                    resets_at: Some(5),
                },
                Window {
                    key: "7d:Opus".into(),
                    pct: 90.0,
                    resets_at: None,
                },
            ],
            source: "engine".into(),
        };
        assert_eq!(o.binding().unwrap().key, "7d:Opus");
        assert_eq!(o.binding_for(Some("claude-sonnet-5-5")).unwrap().key, "7d");
        assert_eq!(o.binding_for(Some("claude-opus-5-5")).unwrap().key, "7d:Opus");
    }

    #[test]
    fn versions_compare() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("2.1.316", "2.1.288"), Greater);
        assert_eq!(compare_versions("2.1.3", "2.1.30"), Less);
        assert_eq!(compare_versions("2.1", "2.1.0"), Equal);
    }

    #[test]
    fn round_trip_transition() {
        let t = Transition {
            id: "t1".into(),
            env: EnvId::default_env(),
            from: Some(AccountId("aaa".into())),
            to: AccountId("bbb".into()),
            how: SwitchHow::Recovery,
            reason: "r".into(),
            state: TransitionState::VerifyingAdoption,
            requested_at: 1,
            written_at: Some(2),
            ended_at: None,
            locked_ms: Some(300),
            detail: None,
            failure: None,
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"how\":\"recovery\""));
        assert_eq!(serde_json::from_str::<Transition>(&s).unwrap(), t);
    }

    #[test]
    fn managed_login_permits() {
        let acct = Account {
            id: AccountId("a".into()),
            label: "x".into(),
            kind: AccountKind::Organization,
            org: "org1".into(),
            subscription: None,
            rate_limit_tier: None,
            registered_at: 0,
            health: Health::Ok,
            unavailable_models: vec![],
            org_name: None,
        };
        let m = ManagedLogin {
            force_org: vec!["org1".into()],
            force_method: None,
            sources: vec![],
        };
        assert!(m.permits(&acct));
        let m2 = ManagedLogin {
            force_org: vec!["org2".into()],
            ..Default::default()
        };
        assert!(!m2.permits(&acct));
        let m3 = ManagedLogin {
            force_method: Some("console".into()),
            ..Default::default()
        };
        assert!(!m3.permits(&acct));
    }
}
