//! Provider access (FR-08): the usage and profile endpoints, the token refresh of idle
//! accounts, and the per-account schedule.
//!
//! A live account's tokens belong to Claude: Quayflow never refreshes them. An idle account's
//! saved login (held only by the vault) is refreshed by the engine when its access token has
//! expired, so its usage stays readable (see `qf_engine` `refresh_idle`).

use qf_model::time::parse_rfc3339_ms;
use qf_model::{AccountId, Observation, Window};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
pub const BETA: &str = "oauth-2025-04-20";

/// The real endpoint, or the same path on the loopback fake server in simulation mode
/// (see [`qf_model::sim`]).
fn endpoint(real: &str) -> String {
    match qf_model::sim::api_base() {
        Some(base) => real.replacen("https://api.anthropic.com", &base, 1),
        None => real.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    /// HTTP status with Retry-After seconds if given.
    Status {
        code: u16,
        retry_after_s: Option<u64>,
    },
    Network(String),
    /// The response did not have the expected shape.
    Shape(String),
    /// Token expired and not refreshed (live: Claude refreshes it; idle: the refresh failed).
    Expired,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Status {
                code: 429,
                retry_after_s,
            } => {
                write!(f, "usage endpoint rate-limited")?;
                if let Some(s) = retry_after_s.filter(|s| *s > 0) {
                    write!(f, " (retry after {s} s)")?;
                }
                Ok(())
            }
            ProviderError::Status { code: 401, .. } => write!(f, "token not accepted (HTTP 401)"),
            ProviderError::Status { code, .. } => write!(f, "HTTP {code}"),
            ProviderError::Network(m) => write!(f, "network: {m}"),
            ProviderError::Shape(m) => write!(f, "unexpected response shape: {m}"),
            ProviderError::Expired => write!(f, "access token expired"),
        }
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let mut b = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("quayflow/", env!("CARGO_PKG_VERSION")));
        if let Ok(tls) = native_tls::TlsConnector::new() {
            b = b.tls_connector(Arc::new(tls));
        }
        b.build()
    })
}

fn get_json(url: &str, access_token: &str) -> Result<serde_json::Value, ProviderError> {
    let r = agent()
        .get(url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", BETA)
        .set("Accept", "application/json")
        .call();
    match r {
        Ok(resp) => resp
            .into_json::<serde_json::Value>()
            .map_err(|e| ProviderError::Shape(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => Err(ProviderError::Status {
            code,
            retry_after_s: resp.header("retry-after").and_then(|v| v.trim().parse().ok()),
        }),
        Err(ureq::Error::Transport(t)) => Err(ProviderError::Network(t.kind().to_string())),
    }
}

/// Parse the usage endpoint response into windows. Extra fields are ignored.
pub fn parse_usage(account: &AccountId, now: i64, v: &serde_json::Value) -> Result<Observation, ProviderError> {
    let mut windows = vec![];
    let window = |key: &str, w: &serde_json::Value| -> Option<Window> {
        let pct = w.get("utilization")?.as_f64()?;
        let resets_at = w.get("resets_at").and_then(|r| r.as_str()).and_then(parse_rfc3339_ms);
        Some(Window {
            key: key.into(),
            pct,
            resets_at,
        })
    };
    if let Some(w) = v
        .get("five_hour")
        .filter(|w| w.is_object())
        .and_then(|w| window("5h", w))
    {
        windows.push(w);
    }
    if let Some(w) = v
        .get("seven_day")
        .filter(|w| w.is_object())
        .and_then(|w| window("7d", w))
    {
        windows.push(w);
    }
    if let Some(limits) = v.get("limits").and_then(|l| l.as_array()) {
        for l in limits {
            let Some(model) = l.pointer("/scope/model/display_name").and_then(|m| m.as_str()) else {
                continue;
            };
            let Some(pct) = l.get("percent").and_then(|p| p.as_f64()) else {
                continue;
            };
            let resets_at = l.get("resets_at").and_then(|r| r.as_str()).and_then(parse_rfc3339_ms);
            windows.push(Window {
                key: format!("7d:{model}"),
                pct,
                resets_at,
            });
        }
    }
    if windows.is_empty() {
        return Err(ProviderError::Shape(
            "no five_hour, seven_day or per-model window".into(),
        ));
    }
    Ok(Observation {
        account: account.clone(),
        ts: now,
        windows,
        source: "engine".into(),
    })
}

/// One usage observation. `expires_at` is the access token's expiry (ms): an expired token is
/// never sent; it is reported as [`ProviderError::Expired`].
pub fn fetch_usage(
    account: &AccountId,
    access_token: &str,
    expires_at: Option<i64>,
    now: i64,
) -> Result<Observation, ProviderError> {
    if expires_at.is_some_and(|e| e < now + 60_000) {
        return Err(ProviderError::Expired);
    }
    let v = get_json(&endpoint(USAGE_URL), access_token)?;
    parse_usage(account, now, &v)
}

/// Claude Code's token service and public OAuth client (the same ones Claude itself uses).
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Scopes asked for when the saved login does not record its own.
pub const DEFAULT_SCOPES: &[&str] = &["user:inference", "user:profile"];

fn token_endpoint() -> String {
    match qf_model::sim::api_base() {
        Some(base) => format!("{base}/v1/oauth/token"),
        None => TOKEN_URL.to_string(),
    }
}

/// New tokens from the token service.
#[derive(Clone, PartialEq)]
pub struct Refreshed {
    pub access_token: String,
    /// The rotated refresh token, or the one sent when the service kept it.
    pub refresh_token: String,
    pub expires_at: i64,
    pub scopes: Option<Vec<String>>,
}

impl std::fmt::Debug for Refreshed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Refreshed {{ expires_at: {} }}", self.expires_at)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefreshError {
    /// The refresh token was refused (`invalid_grant`, 401/403): the login needs a new sign-in.
    Rejected(String),
    /// Network, rate limit or server error: try again later; the saved login is unchanged.
    Failed(ProviderError),
}

impl std::fmt::Display for RefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefreshError::Rejected(m) => write!(f, "refresh token rejected ({m})"),
            RefreshError::Failed(e) => write!(f, "token refresh failed: {e}"),
        }
    }
}

/// Reads the token service's answer. Never includes token text in an error.
pub fn parse_refresh(v: &serde_json::Value, sent_refresh: &str, now: i64) -> Result<Refreshed, RefreshError> {
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| RefreshError::Failed(ProviderError::Shape("no access_token".into())))?;
    let expires_in = v
        .get("expires_in")
        .and_then(|t| t.as_i64())
        .filter(|s| *s > 0)
        .ok_or_else(|| RefreshError::Failed(ProviderError::Shape("no expires_in".into())))?;
    let refresh = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or(sent_refresh);
    let scopes = v
        .get("scope")
        .and_then(|s| s.as_str())
        .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|s| !s.is_empty());
    Ok(Refreshed {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        expires_at: now.saturating_add(expires_in.saturating_mul(1000)),
        scopes,
    })
}

/// Which refusals mean the refresh token itself is dead (never retried).
pub fn refresh_rejected(code: u16, body: &serde_json::Value) -> bool {
    let error = body.get("error").and_then(|e| e.as_str()).unwrap_or_default();
    matches!(code, 401 | 403)
        || (code == 400 && matches!(error, "invalid_grant" | "invalid_client" | "unauthorized_client"))
}

/// Exchanges a refresh token for new tokens. The caller must hold the only copy of this
/// refresh token (it may be rotated) and store the result before using it.
pub fn refresh_access(refresh_token: &str, scopes: &[String], now: i64) -> Result<Refreshed, RefreshError> {
    let scope = if scopes.is_empty() {
        DEFAULT_SCOPES.join(" ")
    } else {
        scopes.join(" ")
    };
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
        "scope": scope,
    });
    let r = agent()
        .post(&token_endpoint())
        .timeout(Duration::from_secs(15))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_json(body);
    match r {
        Ok(resp) => {
            let v = resp
                .into_json::<serde_json::Value>()
                .map_err(|e| RefreshError::Failed(ProviderError::Shape(e.to_string())))?;
            parse_refresh(&v, refresh_token, now)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let retry_after_s = resp.header("retry-after").and_then(|v| v.trim().parse().ok());
            let v = resp.into_json::<serde_json::Value>().unwrap_or_default();
            if refresh_rejected(code, &v) {
                let reason = v.get("error").and_then(|e| e.as_str()).unwrap_or("refused");
                // Only the error code is kept: a description could echo request data.
                Err(RefreshError::Rejected(format!(
                    "HTTP {code} {}",
                    reason
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .take(40)
                        .collect::<String>()
                )))
            } else {
                Err(RefreshError::Failed(ProviderError::Status { code, retry_after_s }))
            }
        }
        Err(ureq::Error::Transport(t)) => Err(RefreshError::Failed(ProviderError::Network(t.kind().to_string()))),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub account_uuid: String,
    pub org_uuid: Option<String>,
    /// Display name or email, for one-time review only. Never stored or logged.
    pub display: Option<String>,
    pub org_type: Option<String>,
}

pub fn parse_profile(v: &serde_json::Value) -> Result<Profile, ProviderError> {
    let uuid = v
        .pointer("/account/uuid")
        .and_then(|u| u.as_str())
        .ok_or_else(|| ProviderError::Shape("profile has no account.uuid".into()))?;
    Ok(Profile {
        account_uuid: uuid.into(),
        org_uuid: v
            .pointer("/organization/uuid")
            .and_then(|u| u.as_str())
            .map(String::from),
        display: v
            .pointer("/account/email")
            .or_else(|| v.pointer("/account/display_name"))
            .or_else(|| v.pointer("/account/full_name"))
            .and_then(|u| u.as_str())
            .map(String::from),
        org_type: v
            .pointer("/organization/organization_type")
            .and_then(|u| u.as_str())
            .map(String::from),
    })
}

/// First-party profile lookup: the identity behind an access token.
pub fn fetch_profile(access_token: &str) -> Result<Profile, ProviderError> {
    parse_profile(&get_json(&endpoint(PROFILE_URL), access_token)?)
}

/// Per-account poll schedule with backoff (live every `live_s`, idle every `idle_s`,
/// 429 backs off by Retry-After or doubling up to `max_backoff_s`). Persisted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub next_at: BTreeMap<String, i64>,
    pub backoff_ms: BTreeMap<String, i64>,
    /// Last reason an account has no fresh observation (sanitized).
    pub note: BTreeMap<String, String>,
    /// After a 429, until when the live account is read at half the pace: the endpoint's
    /// budget is per token and shared with Claude Code itself.
    #[serde(default)]
    pub slow_until: BTreeMap<String, i64>,
}

/// How long a 429 keeps the live account at half the polling pace.
pub const SLOW_FOR_MS: i64 = 60 * 60_000;

/// Seconds to milliseconds, saturating at `i64::MAX`.
fn secs_to_ms(s: u64) -> i64 {
    i64::try_from(s).unwrap_or(i64::MAX).saturating_mul(1000)
}

impl Schedule {
    pub fn due(&self, id: &AccountId, now: i64) -> bool {
        self.next_at.get(id.as_str()).is_none_or(|t| now >= *t)
    }

    pub fn on_success(&mut self, id: &AccountId, now: i64, live: bool, live_s: u64, idle_s: u64) {
        self.backoff_ms.remove(id.as_str());
        self.note.remove(id.as_str());
        let mut every = secs_to_ms(if live { live_s } else { idle_s });
        match self.slow_until.get(id.as_str()) {
            Some(t) if *t > now => {
                if live {
                    every = every.max(secs_to_ms(live_s).saturating_mul(2));
                }
            }
            Some(_) => {
                self.slow_until.remove(id.as_str());
            }
            None => {}
        }
        self.next_at.insert(id.0.clone(), now.saturating_add(every));
    }

    pub fn on_error(&mut self, id: &AccountId, now: i64, err: &ProviderError, live_s: u64, max_backoff_s: u64) {
        self.note.insert(id.0.clone(), err.to_string());
        let wait = match err {
            ProviderError::Status {
                code: 429,
                retry_after_s,
            } => {
                let doubled = self.backoff_ms.get(id.as_str()).copied().unwrap_or(60_000) * 2;
                // A server-supplied Retry-After can be any number: saturate, never overflow.
                let ra = retry_after_s.map(secs_to_ms).unwrap_or(0);
                let w = doubled.max(ra).min(secs_to_ms(max_backoff_s));
                self.backoff_ms.insert(id.0.clone(), w);
                self.slow_until.insert(id.0.clone(), now.saturating_add(SLOW_FOR_MS));
                w
            }
            // Expired token that could not be refreshed now: look again later.
            ProviderError::Expired => 10 * 60_000,
            _ => secs_to_ms(live_s),
        };
        self.next_at.insert(id.0.clone(), now.saturating_add(wait));
    }

    /// Whether a forced read (a person's refresh) may run now: never while a 429
    /// backoff or the slower pace after one has the account waiting.
    pub fn may_force(&self, id: &AccountId, now: i64) -> bool {
        let waiting = self.next_at.get(id.as_str()).is_some_and(|t| now < *t);
        let held =
            self.backoff_ms.contains_key(id.as_str()) || self.slow_until.get(id.as_str()).is_some_and(|t| *t > now);
        !(waiting && held)
    }

    /// Whether the last 429s came in a row (a backoff past its first doubling).
    pub fn repeated_limit(&self, id: &AccountId) -> bool {
        self.backoff_ms.get(id.as_str()).is_some_and(|b| *b > 120_000)
    }

    /// After an engine restart, do not burst: the first poll waits at least a minute
    /// after the last observation.
    pub fn after_restart(&mut self, observations: &[Observation]) {
        for o in observations {
            let t = self.next_at.entry(o.account.0.clone()).or_insert(0);
            *t = (*t).max(o.ts + 60_000);
        }
    }

    /// Make an account due now (account became live; forced refresh).
    pub fn poke(&mut self, id: &AccountId) {
        if !self.backoff_ms.contains_key(id.as_str()) {
            self.next_at.remove(id.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_poc_shape() {
        let v = json!({
            "five_hour": {"utilization": 12.0, "resets_at": "2026-11-01T18:00:00.000+00:00"},
            "seven_day": {"utilization": 72, "resets_at": "2026-11-05T09:00:00+00:00"},
            "seven_day_opus": null,
            "limits": [
                {"scope": {"model": {"display_name": "Opus"}}, "percent": 40, "resets_at": "2026-11-05T09:00:00Z"},
                {"scope": {}, "percent": 99}
            ],
            "extra_usage": {"is_enabled": false}
        });
        let o = parse_usage(&AccountId("a".into()), 5, &v).unwrap();
        assert_eq!(o.windows.len(), 3);
        assert_eq!(o.windows[1].pct, 72.0);
        assert_eq!(o.windows[2].key, "7d:Opus");
        assert!(o.windows[0].resets_at.is_some());
        assert_eq!(o.binding().unwrap().key, "7d");
    }

    #[test]
    fn rejects_unknown_shape() {
        assert!(matches!(
            parse_usage(&AccountId("a".into()), 0, &json!({"x": 1})),
            Err(ProviderError::Shape(_))
        ));
    }

    #[test]
    fn profile_shape() {
        let p =
            parse_profile(&json!({"account": {"uuid": "u", "email": "e@x"}, "organization": {"uuid": "o"}})).unwrap();
        assert_eq!(p.account_uuid, "u");
        assert_eq!(p.org_uuid.as_deref(), Some("o"));
    }

    #[test]
    fn expired_token_is_not_sent() {
        assert_eq!(
            fetch_usage(&AccountId("a".into()), "x", Some(1000), 10_000),
            Err(ProviderError::Expired)
        );
    }

    #[test]
    fn refresh_answer_rotates_or_keeps_the_refresh_token() {
        let r = parse_refresh(
            &json!({"access_token": "A2", "refresh_token": "R2", "expires_in": 28800, "scope": "user:inference user:profile"}),
            "R1",
            1_000,
        )
        .unwrap();
        assert_eq!(r.access_token, "A2");
        assert_eq!(r.refresh_token, "R2");
        assert_eq!(r.expires_at, 1_000 + 28_800_000);
        assert_eq!(
            r.scopes.as_deref(),
            Some(&["user:inference".to_string(), "user:profile".to_string()][..])
        );
        let kept = parse_refresh(&json!({"access_token": "A3", "expires_in": 60}), "R1", 0).unwrap();
        assert_eq!(
            kept.refresh_token, "R1",
            "no refresh_token in the answer: the sent one stays valid"
        );
        assert!(kept.scopes.is_none());
        assert!(matches!(
            parse_refresh(&json!({"expires_in": 60}), "R1", 0),
            Err(RefreshError::Failed(ProviderError::Shape(_)))
        ));
        assert!(!format!("{r:?}").contains("A2"), "Debug never shows tokens");
    }

    #[test]
    fn only_a_dead_refresh_token_is_rejected() {
        assert!(refresh_rejected(400, &json!({"error": "invalid_grant"})));
        assert!(refresh_rejected(401, &json!({})));
        assert!(!refresh_rejected(400, &json!({"error": "invalid_request"})));
        assert!(!refresh_rejected(429, &json!({})));
        assert!(!refresh_rejected(503, &json!({"error": "invalid_grant"})));
    }

    #[test]
    fn schedule_backoff() {
        let id = AccountId("a".into());
        let mut s = Schedule::default();
        assert!(s.due(&id, 0));
        s.on_success(&id, 0, true, 60, 600);
        assert!(!s.due(&id, 59_999));
        assert!(s.due(&id, 60_000));
        let e429 = ProviderError::Status {
            code: 429,
            retry_after_s: None,
        };
        s.on_error(&id, 0, &e429, 60, 900);
        assert_eq!(s.next_at["a"], 120_000);
        s.on_error(&id, 0, &e429, 60, 900);
        assert_eq!(s.next_at["a"], 240_000);
        for _ in 0..10 {
            s.on_error(&id, 0, &e429, 60, 900);
        }
        assert_eq!(s.next_at["a"], 900_000, "capped at 15 min");
        let ra = ProviderError::Status {
            code: 429,
            retry_after_s: Some(3000),
        };
        s.on_error(&id, 0, &ra, 60, 900);
        assert_eq!(s.next_at["a"], 900_000, "Retry-After also capped");
        for huge in [u64::MAX, i64::MAX as u64, i64::MAX as u64 / 1000 + 1] {
            let ra = ProviderError::Status {
                code: 429,
                retry_after_s: Some(huge),
            };
            s.on_error(&id, 0, &ra, 60, 900);
            assert_eq!(s.next_at["a"], 900_000, "huge Retry-After saturates and is capped");
        }
        s.poke(&id);
        assert!(!s.due(&id, 0), "poke does not cut a backoff short");
        s.on_success(&id, 0, false, 60, 600);
        assert_eq!(s.next_at["a"], 600_000);
    }

    #[test]
    fn a_429_halves_the_live_pace_for_an_hour() {
        let id = AccountId("a".into());
        let mut s = Schedule::default();
        let e429 = ProviderError::Status {
            code: 429,
            retry_after_s: Some(0),
        };
        assert_eq!(
            e429.to_string(),
            "usage endpoint rate-limited",
            "no \"retry after 0 s\""
        );
        s.on_error(&id, 0, &e429, 60, 900);
        assert!(!s.repeated_limit(&id));
        assert!(!s.may_force(&id, 60_000), "a forced read waits out the backoff");
        s.on_success(&id, 120_000, true, 60, 600);
        assert_eq!(s.next_at["a"], 240_000, "live every 2 min after a 429");
        assert!(!s.may_force(&id, 200_000), "a forced read keeps the slower pace");
        assert!(s.may_force(&id, 240_000));
        s.on_success(&id, SLOW_FOR_MS + 1, true, 60, 600);
        assert_eq!(
            s.next_at["a"],
            SLOW_FOR_MS + 1 + 60_000,
            "back to every minute after an hour"
        );
        assert!(s.slow_until.is_empty());
        s.on_error(&id, 0, &e429, 60, 900);
        s.on_error(&id, 0, &e429, 60, 900);
        assert!(s.repeated_limit(&id));
    }
}
