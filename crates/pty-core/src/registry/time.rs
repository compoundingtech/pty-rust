//! Timestamps in the shapes the registry writes and reads: ISO-8601 with
//! millisecond precision and a `Z` suffix (`Date.prototype.toISOString`),
//! and local wall-clock `HH:MM:SS` for `pty events` text output.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, now.
pub fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `new Date().toISOString()`: `2026-08-29T10:00:00.123Z`.
pub fn now_iso8601() -> String {
    iso8601_from_epoch_ms(now_epoch_ms())
}

/// Format a `SystemTime` the way `Date.prototype.toISOString` does.
pub fn iso8601(t: SystemTime) -> String {
    let ms = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_else(|e| -(e.duration().as_millis() as i64));
    iso8601_from_epoch_ms(ms)
}

/// Format milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn iso8601_from_epoch_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3_600_000;
    let mm = (rem / 60_000) % 60;
    let ss = (rem / 1000) % 60;
    let mmm = rem % 1000;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{mmm:03}Z")
}

/// Parse an ISO-8601 timestamp (`YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`, the
/// shapes Node writes) into milliseconds since the epoch. `None` for anything
/// else — the caller renders that as `Invalid Date`, as Node does.
pub fn parse_iso8601_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    fn num(b: &[u8], from: usize, len: usize) -> Option<i64> {
        let slice = b.get(from..from + len)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(slice).ok()?.parse().ok()
    }
    let year = num(b, 0, 4)?;
    if b.get(4) != Some(&b'-') {
        return None;
    }
    let month = num(b, 5, 2)?;
    if b.get(7) != Some(&b'-') {
        return None;
    }
    let day = num(b, 8, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut pos = 10;
    let (mut hour, mut minute, mut second, mut milli) = (0, 0, 0, 0);
    if b.get(pos) == Some(&b'T') {
        hour = num(b, 11, 2)?;
        if b.get(13) != Some(&b':') {
            return None;
        }
        minute = num(b, 14, 2)?;
        pos = 16;
        if b.get(pos) == Some(&b':') {
            second = num(b, 17, 2)?;
            pos = 19;
            if b.get(pos) == Some(&b'.') {
                let start = pos + 1;
                let mut end = start;
                while b.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                if end == start {
                    return None;
                }
                let frac = &s[start..end];
                let scaled = format!("{:0<3}", frac);
                milli = scaled[..3].parse().ok()?;
                pos = end;
            }
        }
        if hour > 24 || minute > 59 || second > 59 {
            return None;
        }
    }
    let mut offset_ms = 0i64;
    match b.get(pos) {
        None => {
            // Date-only forms are UTC; date-time forms without a zone are local
            // time in JS. Treat both as UTC: the registry always writes `Z`.
        }
        Some(b'Z') => {
            if pos + 1 != b.len() {
                return None;
            }
        }
        Some(sign @ (b'+' | b'-')) => {
            let oh = num(b, pos + 1, 2)?;
            if b.get(pos + 3) != Some(&b':') {
                return None;
            }
            let om = num(b, pos + 4, 2)?;
            if pos + 6 != b.len() {
                return None;
            }
            offset_ms = (oh * 60 + om) * 60_000;
            if *sign == b'+' {
                offset_ms = -offset_ms;
            }
        }
        Some(_) => return None,
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000 + milli + offset_ms)
}

/// Local wall-clock `HH:MM:SS` for a millisecond epoch timestamp
/// (`toLocaleTimeString("en-US", { hour12: false })`).
pub fn local_hms(epoch_ms: i64) -> String {
    let secs = epoch_ms.div_euclid(1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` fills the caller-provided struct and reads only
    // the pointed-to time value.
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
        return "Invalid Date".to_string();
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// `SystemTime` for a millisecond epoch value (for tests and callers that
/// want to compare against `SystemTime::now()`).
pub fn system_time_from_epoch_ms(ms: i64) -> SystemTime {
    if ms >= 0 {
        UNIX_EPOCH + Duration::from_millis(ms as u64)
    } else {
        UNIX_EPOCH - Duration::from_millis((-ms) as u64)
    }
}

// Howard Hinnant's civil calendar algorithms (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trips() {
        let s = "2026-04-05T10:15:03.000Z";
        let ms = parse_iso8601_ms(s).unwrap();
        assert_eq!(iso8601_from_epoch_ms(ms), s);
        assert_eq!(iso8601_from_epoch_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(parse_iso8601_ms("2026-04-05T00:00:1049Z"), None);
        assert_eq!(parse_iso8601_ms("t"), None);
        assert_eq!(
            parse_iso8601_ms("2026-04-05T12:00:00+02:00"),
            parse_iso8601_ms("2026-04-05T10:00:00Z")
        );
    }
}
