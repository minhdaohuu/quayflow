//! Provider access (FR-08): the usage and profile endpoints, and the per-account schedule.
//!
//! Quayflow never refreshes a token and never calls the token service. An idle account
//! whose access token expired is reported as stale until Claude refreshes it while live.

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
    /// Token expired: not observed (Quayflow does not refresh).
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
                if let Some(s) = retry_after_s {
                    write!(f, " (retry after {s} s)")?;
                }
                Ok(())
            }
            ProviderError::Status { code: 401, .. } => write!(f, "token not accepted (HTTP 401)"),
            ProviderError::Status { code, .. } => write!(f, "HTTP {code}"),
            ProviderError::Network(m) => write!(f, "network: {m}"),
            ProviderError::Shape(m) => write!(f, "unexpected response shape: {m}"),
            ProviderError::Expired => write!(
                f,
                "access token expired while idle; refreshes when this account is live"
            ),
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
}

impl Schedule {
    pub fn due(&self, id: &AccountId, now: i64) -> bool {
        self.next_at.get(id.as_str()).is_none_or(|t| now >= *t)
    }

    pub fn on_success(&mut self, id: &AccountId, now: i64, live: bool, live_s: u64, idle_s: u64) {
        self.backoff_ms.remove(id.as_str());
        self.note.remove(id.as_str());
        let every = if live { live_s } else { idle_s } as i64 * 1000;
        self.next_at.insert(id.0.clone(), now + every);
    }

    pub fn on_error(&mut self, id: &AccountId, now: i64, err: &ProviderError, live_s: u64, max_backoff_s: u64) {
        self.note.insert(id.0.clone(), err.to_string());
        let wait = match err {
            ProviderError::Status {
                code: 429,
                retry_after_s,
            } => {
                let doubled = self.backoff_ms.get(id.as_str()).copied().unwrap_or(60_000) * 2;
                let ra = retry_after_s.map(|s| s as i64 * 1000).unwrap_or(0);
                let w = doubled.max(ra).min(max_backoff_s as i64 * 1000);
                self.backoff_ms.insert(id.0.clone(), w);
                w
            }
            // Expired idle token: look again later; it changes when the account is live.
            ProviderError::Expired => 10 * 60_000,
            _ => live_s as i64 * 1000,
        };
        self.next_at.insert(id.0.clone(), now + wait);
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
        s.poke(&id);
        assert!(!s.due(&id, 0), "poke does not cut a backoff short");
        s.on_success(&id, 0, false, 60, 600);
        assert_eq!(s.next_at["a"], 600_000);
    }
}
