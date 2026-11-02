//! Loopback API v1 schema shared by the engine, the CLI and (by documentation) the Mod.
//! All bodies are JSON with camelCase keys. Every request carries `x-quayflow-token`.
//! Mutating requests may carry `requestId`; a repeated id returns the first response.

use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloRequest {
    pub protocol: u32,
    #[serde(default)]
    pub mod_version: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub claude_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResponse {
    pub compatible: bool,
    pub protocol: u32,
    pub accepted_protocols: Vec<u32>,
    pub engine_version: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: String,
    /// "info" | "warn" | "error"
    pub level: String,
    /// Machine kind, for example "unknown_login", "other_switcher", "paused", "incompatible_version",
    /// "managed_login", "d_org_enabled", "group_conflict", "adoption_mismatch", "vault_error",
    /// "refresh_rejected", "suggestion".
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub persistent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    #[serde(default)]
    pub binding: Option<Window>,
    pub windows: Vec<Window>,
    pub age_s: i64,
    pub fresh: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eligibility {
    pub eligible: bool,
    /// Why not (or why it is the pick), with timestamps where relevant.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: AccountId,
    pub label: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub subscription: Option<String>,
    pub health: Health,
    pub groups: Vec<String>,
    /// Environments where this account is the live login.
    pub live_in: Vec<EnvId>,
    #[serde(default)]
    pub usage: Option<UsageView>,
    /// Why there is no usage (stale token, rate-limited endpoint, not observed yet).
    #[serde(default)]
    pub usage_note: Option<String>,
    #[serde(default)]
    pub eligibility: Option<Eligibility>,
    #[serde(default)]
    pub unavailable_models: Vec<String>,
    #[serde(default)]
    pub rank: Option<Rank>,
    /// Opaque organization hash and its local display name.
    pub org: String,
    #[serde(default)]
    pub org_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    #[serde(flatten)]
    pub group: Group,
    pub is_default: bool,
    pub mixed: bool,
    pub mixed_covered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveLogin {
    #[serde(default)]
    pub account: Option<AccountId>,
    pub known: bool,
    #[serde(default)]
    pub label: Option<String>,
    /// No login at all in the environment (signed out).
    #[serde(default)]
    pub signed_out: bool,
    #[serde(default)]
    pub kind: Option<AccountKind>,
    #[serde(default)]
    pub subscription: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InPlace {
    pub allowed: bool,
    pub reason: String,
    /// Oldest running Claude version governing this environment.
    #[serde(default)]
    pub governing_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvView {
    pub id: EnvId,
    pub config_dir: String,
    pub live: LiveLogin,
    /// Effective mode (most restrictive of the governing groups).
    pub mode: Mode,
    pub governing_groups: Vec<String>,
    pub in_place: InPlace,
    pub managed: ManagedLogin,
    /// Display names of the enforced organizations, where a registered account belongs to them.
    #[serde(default)]
    pub managed_org_names: Vec<String>,
    #[serde(default)]
    pub transition: Option<Transition>,
    #[serde(default)]
    pub last_switch: Option<Transition>,
    #[serde(default)]
    pub pin: Option<Pin>,
    #[serde(default)]
    pub suggestion: Option<Suggestion>,
    pub adoption: Vec<ClientAdoption>,
    pub sessions: Vec<SessionInfo>,
    /// Current decision explanation.
    pub decision: String,
    pub automation_paused: bool,
    #[serde(default)]
    pub paused_reason: Option<String>,
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub session_id: String,
    pub env: EnvId,
    pub group: String,
    /// "assigned" (trusted project assignment) or "default".
    pub group_source: String,
    /// Account this session's last verified request used.
    #[serde(default)]
    pub verified_account: Option<AccountId>,
    #[serde(default)]
    pub verified_at: Option<i64>,
    #[serde(default)]
    pub adoption: Option<ClientAdoption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub version: String,
    pub protocol: u32,
    pub pid: u32,
    pub started_at: i64,
    pub data_dir: String,
    /// "not code-signed; provenance-attested" plus the executable's SHA-256.
    pub integrity: String,
    pub exe_sha256: String,
    /// Set only in simulation mode (`QUAYFLOW_SIM_API`): the fake API origin in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub ts: i64,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneView {
    pub name: String,
    pub config_dir: String,
    #[serde(default)]
    pub live: Option<AccountId>,
    pub links_ok: bool,
    pub link_detail: Vec<String>,
    pub canary_passed: bool,
    pub sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub engine: EngineInfo,
    pub onboarded: bool,
    pub consents: Consents,
    #[serde(default)]
    pub session: Option<SessionView>,
    pub envs: Vec<EnvView>,
    pub accounts: Vec<AccountView>,
    pub groups: Vec<GroupView>,
    pub default_group: String,
    pub projects: Vec<ProjectAssignment>,
    pub tuning: Tuning,
    pub compat: Vec<CompatRecord>,
    pub d_org: DOrgSetting,
    pub lanes: Vec<LaneView>,
    pub activity: Vec<ActivityItem>,
    /// Event sequence number at the time of this snapshot (for /v1/events?since=).
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineEvent {
    pub seq: u64,
    pub ts: i64,
    /// "state" | "suggestion" | "switched" | "adoption" | "notice" | "transition"
    pub kind: String,
    #[serde(default)]
    pub env: Option<EnvId>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsResponse {
    pub seq: u64,
    pub events: Vec<EngineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRequest {
    /// Account id or label.
    pub to: String,
    #[serde(default)]
    pub env: Option<EnvId>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Prefer a moment with no busy session (bounded by idleWaitS).
    #[serde(default)]
    pub idle_wait: bool,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResponse {
    pub ok: bool,
    #[serde(default)]
    pub transition: Option<Transition>,
    /// Why it was refused, or a note ("already live").
    #[serde(default)]
    pub message: Option<String>,
    /// Session that blocks the switch (group conflict), for the lane offer.
    #[serde(default)]
    pub blocked_by: Option<SessionInfo>,
}

/// Rate-limit entry as `$.session.usage().rateLimits` reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRateLimit {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub percent_used: Option<f64>,
    /// Epoch ms or RFC 3339 as the client reports it.
    #[serde(default)]
    pub resets_at: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub ok: bool,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub rate_limits: Vec<SessionRateLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StepResponse {
    /// The Mod should read the identity its last request used and report it (/v1/adoption).
    pub verify: bool,
    #[serde(default)]
    pub transition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptionReport {
    pub session_id: String,
    /// Identity hash the Mod computed from the profile lookup; None when the lookup failed.
    #[serde(default)]
    pub account: Option<AccountId>,
    #[serde(default)]
    pub transition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecoverRequest {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub simulated: bool,
    #[serde(default)]
    pub rate_limits: Vec<SessionRateLimit>,
    /// Sanitized error class if the Mod could read one ("model_not_found", "rate_limit", ...).
    #[serde(default)]
    pub error_kind: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecoverResponse {
    /// Retry the same request once.
    pub retry: bool,
    pub switched: bool,
    #[serde(default)]
    pub to: Option<AccountId>,
    #[serde(default)]
    pub to_label: Option<String>,
    pub why: String,
    #[serde(default)]
    pub transition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FailureShape {
    pub session_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub answer_empty: bool,
    #[serde(default)]
    pub tool_uses: u32,
    #[serde(default)]
    pub has_usage: bool,
    #[serde(default)]
    pub error_kind: Option<String>,
    /// Top-level field names of the result (no values).
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub rate_limits: Vec<SessionRateLimit>,
    #[serde(default)]
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OkResponse {
    pub ok: bool,
    #[serde(default)]
    pub message: Option<String>,
}

impl OkResponse {
    pub fn ok(msg: impl Into<String>) -> Self {
        OkResponse {
            ok: true,
            message: Some(msg.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub error: String,
    /// Machine code: "bad_token", "invalid", "not_found", "conflict", "refused", "incompatible", "internal".
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigninStatus {
    pub id: String,
    /// "waiting" | "ready" | "failed" | "cancelled" | "saved"
    pub state: String,
    pub config_dir: String,
    /// The command the user can run when no window could be opened.
    pub command: String,
    #[serde(default)]
    pub identity: Option<SigninIdentity>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigninIdentity {
    pub account: AccountId,
    pub kind: AccountKind,
    #[serde(default)]
    pub subscription: Option<String>,
    /// Already registered under this label (duplicate).
    #[serde(default)]
    pub duplicate_of: Option<String>,
    /// Reauthentication expected a different identity.
    #[serde(default)]
    pub unexpected: bool,
    /// Display name from the profile, shown once for review; never stored.
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChangeResponse {
    pub ok: bool,
    /// The change would mix identities and needs the one-time disclosure accepted.
    #[serde(default)]
    pub needs_disclosure: bool,
    #[serde(default)]
    pub disclosure: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffResponse {
    pub ok: bool,
    pub file_count: usize,
    /// Exact command that resumes the session under the destination environment.
    pub command: String,
    #[serde(default)]
    pub launched: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportResponse {
    pub path: String,
    pub preview: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recover_request_minimal() {
        let r: RecoverRequest =
            serde_json::from_str(r#"{"sessionId":"s","rateLimits":[{"kind":"five_hour","percentUsed":100}]}"#).unwrap();
        assert_eq!(r.rate_limits[0].percent_used, Some(100.0));
        assert!(!r.simulated);
    }
}
