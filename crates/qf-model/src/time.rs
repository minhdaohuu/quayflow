//! Time helpers: epoch milliseconds everywhere.

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parse an RFC 3339 timestamp ("2026-11-01T18:00:00.123+00:00") to epoch milliseconds.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

pub fn to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Local calendar day "YYYYMMDD" for log file names.
pub fn day_stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y%m%d").to_string())
        .unwrap_or_else(|| "00000000".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01.5+00:00"), Some(1500));
        assert_eq!(parse_rfc3339_ms("nope"), None);
        assert_eq!(to_rfc3339(1000), "1970-01-01T00:00:01Z");
    }
}
