//! Simulation mode for tests: `QUAYFLOW_SIM_API` points the usage and profile calls at a local
//! fake server instead of `https://api.anthropic.com`.
//!
//! Only a loopback HTTP origin is accepted (`http://127.0.0.1:<port>` or
//! `http://localhost:<port>`, no path), so the setting can never send a login token to another
//! host. Any other value is ignored and the real endpoints are used.

/// The validated fake API origin, if simulation mode is on.
pub fn api_base() -> Option<String> {
    let raw = std::env::var("QUAYFLOW_SIM_API").ok()?;
    parse_base(raw.trim())
}

/// True when a valid `QUAYFLOW_SIM_API` is set.
pub fn active() -> bool {
    api_base().is_some()
}

fn parse_base(v: &str) -> Option<String> {
    let v = v.trim_end_matches('/');
    let rest = v.strip_prefix("http://")?;
    let (host, port) = rest.split_once(':')?;
    if !matches!(host, "127.0.0.1" | "localhost") {
        return None;
    }
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let p: u32 = port.parse().ok()?;
    if p == 0 || p > 65535 {
        return None;
    }
    Some(format!("http://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::parse_base;

    #[test]
    fn accepts_only_loopback_http_origins() {
        assert_eq!(
            parse_base("http://127.0.0.1:8123").as_deref(),
            Some("http://127.0.0.1:8123")
        );
        assert_eq!(parse_base("http://localhost:9/").as_deref(), Some("http://localhost:9"));
        for bad in [
            "https://127.0.0.1:8123",
            "http://127.0.0.1",
            "http://127.0.0.1:8123/api",
            "http://example.com:80",
            "http://127.0.0.1.example.com:80",
            "http://user@127.0.0.1:80",
            "http://[::1]:80",
            "http://127.0.0.1:0",
            "http://127.0.0.1:70000",
            "",
        ] {
            assert_eq!(parse_base(bad), None, "{bad}");
        }
    }
}
