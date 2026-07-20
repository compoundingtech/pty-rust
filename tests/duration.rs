//! Port of the pty project's `tests/duration.test.ts`.

use pty_testkit::duration::{format_duration, parse_duration};

// ── parse_duration ──

#[test]
fn parses_seconds() {
    assert_eq!(parse_duration("30s"), Some(30_000));
    assert_eq!(parse_duration("1s"), Some(1000));
    assert_eq!(parse_duration("0s"), Some(0));
}

#[test]
fn parses_minutes() {
    assert_eq!(parse_duration("5m"), Some(5 * 60_000));
    assert_eq!(parse_duration("1m"), Some(60_000));
}

#[test]
fn parses_hours() {
    assert_eq!(parse_duration("2h"), Some(2 * 3_600_000));
}

#[test]
fn parses_days() {
    assert_eq!(parse_duration("7d"), Some(7 * 86_400_000));
}

#[test]
fn is_case_insensitive_on_unit() {
    assert_eq!(parse_duration("2H"), Some(2 * 3_600_000));
    assert_eq!(parse_duration("30S"), Some(30_000));
}

#[test]
fn tolerates_whitespace() {
    assert_eq!(parse_duration("  5m  "), Some(5 * 60_000));
    assert_eq!(parse_duration("5 m"), Some(5 * 60_000));
}

#[test]
fn rejects_compound_forms() {
    assert_eq!(parse_duration("1h30m"), None);
    assert_eq!(parse_duration("1h 30m"), None);
}

#[test]
fn rejects_missing_unit_or_number() {
    assert_eq!(parse_duration("5"), None);
    assert_eq!(parse_duration("s"), None);
    assert_eq!(parse_duration(""), None);
}

#[test]
fn rejects_unknown_units() {
    assert_eq!(parse_duration("5y"), None);
    assert_eq!(parse_duration("5w"), None);
    assert_eq!(parse_duration("5ms"), None);
}

#[test]
fn rejects_negative_and_non_integer() {
    assert_eq!(parse_duration("-5m"), None);
    assert_eq!(parse_duration("1.5h"), None);
}

// ── format_duration ──

#[test]
fn renders_sub_minute_in_seconds() {
    assert_eq!(format_duration(0), "0s");
    assert_eq!(format_duration(1000), "1s");
    assert_eq!(format_duration(45_000), "45s");
    assert_eq!(format_duration(59_999), "59s");
}

#[test]
fn renders_sub_hour_in_minutes() {
    assert_eq!(format_duration(60_000), "1m");
    assert_eq!(format_duration(65_000), "1m5s");
    assert_eq!(format_duration(30 * 60_000), "30m");
}

#[test]
fn renders_sub_day_in_hours() {
    assert_eq!(format_duration(3_600_000), "1h");
    assert_eq!(format_duration(3_600_000 + 12 * 60_000), "1h12m");
    assert_eq!(format_duration(23 * 3_600_000), "23h");
}

#[test]
fn renders_multi_day_in_days_hours() {
    assert_eq!(format_duration(86_400_000), "1d");
    assert_eq!(format_duration(86_400_000 + 2 * 3_600_000), "1d2h");
    assert_eq!(format_duration(3 * 86_400_000), "3d");
}

#[test]
fn treats_negative_as_zero() {
    assert_eq!(format_duration(-1000), "0s");
}
