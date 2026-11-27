//! Strict UTC timestamp parsing and formatting for bi-temporal support.
//!
//! Accepts Gregorian dates and UTC datetimes with four-digit years 0000–9999.
//! Fractions of 1–9 ASCII digits are truncated to milliseconds.

use crate::error::{ErrorCode, err_coded};
use anyhow::Result;

const MIN_MILLIS: i64 = -62_167_219_200_000;
const MAX_MILLIS: i64 = 253_402_300_799_999;
const MILLIS_PER_DAY: i64 = 86_400_000;
// Days from 0000-01-01 to 1970-01-01 in the proleptic Gregorian calendar.
const EPOCH_DAYS: i64 = 719_528;

fn leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn month_days(year: i64, month: i64) -> i64 {
    match month {
        2 if leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Count leap years in [0, year): year zero is itself a leap year.
// All callers supply a year in 0..=10000, so these operations cannot overflow.
fn days_before_year(year: i64) -> i64 {
    365 * year + (year + 3) / 4 - (year + 99) / 100 + (year + 399) / 400
}

/// Parse `YYYY-MM-DD` (midnight UTC), `YYYY-MM-DDTHH:MM:SSZ`, or
/// `YYYY-MM-DDTHH:MM:SS.fZ` (1–9 fractional digits) to Unix milliseconds.
///
/// Fields must have exactly the specified widths and use ASCII digits, uppercase
/// `T`/`Z`, years 0000–9999 and seconds 00–59. No whitespace or offsets are allowed.
/// Sub-millisecond digits are discarded, including before the Unix epoch.
pub fn parse_timestamp(s: &str) -> Result<i64> {
    let bytes = s.as_bytes();
    let invalid = |reason: &str| {
        err_coded!(
            ErrorCode::Int004,
            format!("invalid UTC timestamp '{s}': {reason}")
        )
    };
    if bytes.contains(&b'+') || bytes.get(10..).is_some_and(|rest| rest.contains(&b'-')) {
        return Err(invalid(
            "timezone offsets are not supported; use UTC (Z) timestamps only",
        ));
    }
    if !matches!(bytes.len(), 10 | 20 | 22..=30)
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
    {
        return Err(invalid(
            "expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS[.f]Z (1–9 fractional digits)",
        ));
    }
    // Slice bytes, never UTF-8 strings: malformed multibyte input at any offset
    // is an error, not a panic. Each numeric field is bounded to at most 4 digits.
    let field = |start, end, name| -> Result<i64> {
        let digits = bytes.get(start..end).ok_or_else(|| invalid(name))?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return Err(invalid(name));
        }
        Ok(digits
            .iter()
            .fold(0, |n, digit| n * 10 + i64::from(digit - b'0')))
    };
    let year = field(0, 4, "year must contain four ASCII digits")?;
    let month = field(5, 7, "month must contain two ASCII digits")?;
    let day = field(8, 10, "day must contain two ASCII digits")?;
    if !(1..=12).contains(&month) {
        return Err(invalid("month must be in 01–12"));
    }
    if !(1..=month_days(year, month)).contains(&day) {
        return Err(invalid("day is out of range for this month and year"));
    }
    let days = days_before_year(year) + (1..month).map(|m| month_days(year, m)).sum::<i64>() + day
        - 1
        - EPOCH_DAYS;
    let midnight = days * MILLIS_PER_DAY;
    if bytes.len() == 10 {
        return Ok(midnight);
    }
    if bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes.last() != Some(&b'Z')
    {
        return Err(invalid("expected uppercase T, HH:MM:SS and uppercase Z"));
    }
    let hour = field(11, 13, "hour must contain two ASCII digits")?;
    let minute = field(14, 16, "minute must contain two ASCII digits")?;
    let second = field(17, 19, "second must contain two ASCII digits")?;
    if hour > 23 {
        return Err(invalid("hour must be in 00–23"));
    }
    if minute > 59 {
        return Err(invalid("minute must be in 00–59"));
    }
    if second > 59 {
        return Err(invalid(
            "second must be in 00–59; leap seconds are not supported",
        ));
    }
    let mut fraction = 0;
    if bytes.len() > 20 {
        if bytes.get(19) != Some(&b'.') {
            return Err(invalid("fraction must start with a decimal point"));
        }
        let digits = bytes
            .get(20..bytes.len() - 1)
            .ok_or_else(|| invalid("missing fraction"))?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return Err(invalid("fraction must contain 1–9 ASCII digits"));
        }
        // Pad short fractions to three digits; ignore further precision. Adding
        // this nonnegative fraction to the whole second also floors pre-epoch times.
        for digit in digits
            .iter()
            .copied()
            .chain(std::iter::repeat(b'0'))
            .take(3)
        {
            fraction = fraction * 10 + i64::from(digit - b'0');
        }
    }
    Ok(midnight + (hour * 3600 + minute * 60 + second) * 1000 + fraction)
}

/// Format Unix milliseconds as `YYYY-MM-DDTHH:MM:SSZ`, flooring to seconds.
///
/// Returns INT-005 outside 0000-01-01T00:00:00Z through
/// 9999-12-31T23:59:59.999Z. Check the `VALID_TIME_FOREVER` sentinel before calling.
#[allow(dead_code)]
pub fn millis_to_timestamp_string(millis: i64) -> Result<String> {
    if !(MIN_MILLIS..=MAX_MILLIS).contains(&millis) {
        return Err(err_coded!(ErrorCode::Int005, millis));
    }
    let days = millis.div_euclid(MILLIS_PER_DAY) + EPOCH_DAYS;
    // Find the year in at most 14 comparisons, with a fixed exclusive upper
    // bound. This uses the same year-zero/leap-day convention as the parser.
    let (mut year, mut end) = (0, 10_000);
    while year + 1 < end {
        let middle = (year + end) / 2;
        if days_before_year(middle) <= days {
            year = middle;
        } else {
            end = middle;
        }
    }
    let mut day = days - days_before_year(year);
    let mut month = 1;
    while day >= month_days(year, month) {
        day -= month_days(year, month);
        month += 1;
    }
    let day = day + 1;
    let seconds = millis.rem_euclid(MILLIS_PER_DAY) / 1000;
    let (hour, minute, second) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

#[cfg(test)]
mod chrono_vectors;

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
        assert!(
            err.to_string()
                .contains("timezone offsets are not supported")
        );
        assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-004");
    }

    #[test]
    fn test_reject_invalid_string() {
        assert!(parse_timestamp("not-a-date").is_err());
    }

    #[test]
    fn test_multibyte_char_near_byte_offset_10_does_not_panic() {
        // Byte offset 10 falls inside the 2-byte UTF-8 encoding of '\u{605}',
        // which starts at byte 9. Slicing s[10..] would panic on a non-char-boundary.
        let s = "123456789\u{605}abc";
        assert!(parse_timestamp(s).is_err());
    }

    #[test]
    fn test_millis_to_timestamp_roundtrip() {
        let original = "2024-01-15T10:00:00Z";
        let millis = parse_timestamp(original).unwrap();
        let back = millis_to_timestamp_string(millis).unwrap();
        assert_eq!(back, original);
    }

    fn assert_invalid(input: &str) {
        let err = parse_timestamp(input).expect_err(input);
        let coded = crate::error::MinigrafError::from(err);
        assert_eq!(coded.code(), "INT-004", "input: {input}");
    }

    #[test]
    fn test_reject_non_strict_forms() {
        for input in [
            "2024-01-15T10:00:00z",
            "2024-01-15T10:00:00UTC",
            "2024-01-15T10:00:00 UTC",
            "2024-01-15T10:00:00 Z",
            " 2023-06-01",
            "2023-06-01 ",
            "2023-06-01\n",
            "2024-01-15T10:00:00Z ",
            "\t2024-01-15T10:00:00Z",
            "2023-6-1",
            "2023-06-1",
            "2023-6-01",
            "2024-01-15T1:00:00Z",
            "2024-01-15T10:0:00Z",
            "2024-01-15T10:00:0Z",
            "24-01-15",
            "23-06-01T10:00:00Z",
            "1-01-01",
            "1-01-01T00:00:00Z",
            "-0001-01-01",
            "-1-01-01",
            "2024-01-15T23:59:60Z",
            "2024-01-15T10:00:00.1234567890Z",
            "2024-01-15T10:00:00.Z",
            "2024-01-15T10:00:00,1Z",
            "2024-01-15T10:00:00+05:30",
            "2024-01-15T10:00:00-00:00",
            "2024-01-15t10:00:00Z",
            "2024-01-15 10:00:00Z",
            "2024-01-15T10:00Z",
            "10000-01-01",
            "10000-01-01T00:00:00Z",
            "2024-01-15T24:00:00Z",
            "2023-02-29",
            "",
            "２０２４-01-15",
            "2024-01-15T10:00:00.١Z",
            "2023-06-01x",
            "2024-01-15T10:00:00Zx",
            "2024-01-15T10:00:00",
            "+2024-01-15",
        ] {
            assert_invalid(input);
        }
    }

    #[test]
    fn test_calendar_and_clock_boundaries() {
        for input in [
            "2024-00-01",
            "2024-13-01",
            "2024-01-00",
            "2024-01-32",
            "2024-04-31",
            "2024-06-31",
            "2024-09-31",
            "2024-11-31",
            "1900-02-29",
            "2100-02-29",
            "2000-02-30",
            "0001-02-29",
            "2024-01-15T00:60:00Z",
            "2024-01-15T00:00:60Z",
        ] {
            assert_invalid(input);
        }
        for (input, expected) in [
            ("0000-01-01T00:00:00Z", -62_167_219_200_000),
            ("9999-12-31T23:59:59Z", 253_402_300_799_000),
            ("0000-02-29", -62_162_121_600_000),
            ("1900-03-01", -2_203_891_200_000),
            ("2000-02-29", 951_782_400_000),
            ("1970-01-01T00:00:00Z", 0),
            ("1969-12-31T23:59:59.9999Z", -1),
        ] {
            assert_eq!(parse_timestamp(input).unwrap(), expected, "input: {input}");
        }
    }

    #[test]
    fn test_chrono_reference_vectors() {
        assert_eq!(chrono_vectors::CHRONO_VECTORS.len(), 328);
        for &(input, millis) in chrono_vectors::CHRONO_VECTORS {
            assert_eq!(parse_timestamp(input).unwrap(), millis, "input: {input}");
            if input.contains('T') && millis.rem_euclid(1000) == 0 {
                let expected = format!("{}Z", &input[..19]);
                assert_eq!(millis_to_timestamp_string(millis).unwrap(), expected);
            }
        }
    }

    #[test]
    fn test_format_range_edges() {
        for (millis, expected) in [
            (-62_167_219_200_000, "0000-01-01T00:00:00Z"),
            (253_402_300_799_999, "9999-12-31T23:59:59Z"),
            (-1, "1969-12-31T23:59:59Z"),
        ] {
            assert_eq!(millis_to_timestamp_string(millis).unwrap(), expected);
        }
        for millis in [-62_167_219_200_001, 253_402_300_800_000, i64::MIN, i64::MAX] {
            let err = millis_to_timestamp_string(millis).unwrap_err();
            assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-005");
        }
    }

    #[test]
    fn test_multibyte_at_every_position() {
        let valid = "2024-01-15T10:00:00.123456789Z";
        for pos in 0..=valid.len() {
            for ch in ['é', '中', '🦀'] {
                let mut input = valid.to_owned();
                input.insert(pos, ch);
                assert_invalid(&input);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod properties {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn formatting_round_trips_to_whole_second(millis in -62_167_219_200_000i64..=253_402_300_799_999) {
                let text = millis_to_timestamp_string(millis).unwrap();
                prop_assert_eq!(parse_timestamp(&text).unwrap(), millis.div_euclid(1000) * 1000);
            }

            #[test]
            fn parsing_arbitrary_strings_never_panics(input in any::<String>()) {
                let _ = parse_timestamp(&input);
            }
        }
    }
}
