//! Timestamp parsing utilities for bi-temporal support.
//!
//! Supports UTC ISO 8601 strings only. Timezone offsets are rejected
//! to avoid chrono's local timezone handling (GHSA-wcg3-cvx6-7396).

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

/// Parse an ISO 8601 UTC string to milliseconds since UNIX epoch.
///
/// Accepted formats:
/// - `"2024-01-15T10:00:00Z"` — UTC datetime
/// - `"2023-06-01"` — date only, interpreted as midnight UTC
///
/// Rejected:
/// - Any string with a timezone offset (e.g., `+05:30`) — use UTC (Z) only.
pub fn parse_timestamp(s: &str) -> Result<i64> {
    // Reject timezone offsets explicitly
    if s.contains('+') || (s.len() > 10 && s[10..].contains('-')) {
        return Err(anyhow!(
            "timezone offsets are not supported; use UTC (Z) timestamps only. \
             chrono's local timezone handling (GHSA-wcg3-cvx6-7396) is avoided by design."
        ));
    }

    // Try full datetime first
    if s.contains('T') {
        let dt = s.parse::<DateTime<Utc>>()
            .map_err(|e| anyhow!("invalid UTC timestamp '{}': {}", s, e))?;
        return Ok(dt.timestamp_millis());
    }

    // Try date-only (YYYY-MM-DD)
    let date = s.parse::<NaiveDate>()
        .map_err(|e| anyhow!("invalid date '{}': {}", s, e))?;
    let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
    Ok(dt.timestamp_millis())
}

/// Convert milliseconds since UNIX epoch back to a UTC ISO 8601 string.
///
/// Returns an error if `millis` is outside chrono's supported range.
/// Note: `i64::MAX` (VALID_TIME_FOREVER) should never be passed to this function;
/// callers should check for the sentinel before formatting.
pub fn millis_to_timestamp_string(millis: i64) -> Result<String> {
    let dt = DateTime::<Utc>::from_timestamp_millis(millis)
        .ok_or_else(|| anyhow!("millisecond value {} is outside the supported datetime range", millis))?;
    Ok(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_utc_datetime() {
        let ts = parse_timestamp("2024-01-15T10:00:00Z").unwrap();
        assert_eq!(ts, 1705312800000_i64);
    }

    #[test]
    fn test_parse_date_only() {
        let ts = parse_timestamp("2023-06-01").unwrap();
        // 2023-06-01 midnight UTC
        assert_eq!(ts, 1685577600000_i64);
    }

    #[test]
    fn test_reject_timezone_offset() {
        let err = parse_timestamp("2024-01-15T10:00:00+05:30").unwrap_err();
        assert!(err.to_string().contains("timezone offsets are not supported"));
        assert!(err.to_string().contains("GHSA-wcg3-cvx6-7396"));
    }

    #[test]
    fn test_reject_invalid_string() {
        assert!(parse_timestamp("not-a-date").is_err());
    }

    #[test]
    fn test_millis_to_timestamp_roundtrip() {
        let original = "2024-01-15T10:00:00Z";
        let millis = parse_timestamp(original).unwrap();
        let back = millis_to_timestamp_string(millis).unwrap();
        assert_eq!(back, original);
    }
}
