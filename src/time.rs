//! Strict RFC 3339 parsing and UTC formatting, without a date library.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn digits(bytes: &[u8], from: usize, len: usize) -> Option<u32> {
    let part = bytes.get(from..from + len)?;
    let mut value = 0u32;
    for b in part {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(b - b'0');
    }
    Some(value)
}

/// Parses an RFC 3339 timestamp (`2026-11-23T21:04:05.123Z` or with an offset) into
/// nanoseconds since the Unix epoch. Anything else, Unix seconds and zone-less times
/// included, gives `None`.
pub(crate) fn parse_rfc3339_nanos(input: &str) -> Option<i128> {
    let b = input.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let year = i64::from(digits(b, 0, 4)?);
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let month = digits(b, 5, 2)?;
    let day = digits(b, 8, 2)?;
    let hour = digits(b, 11, 2)?;
    let minute = digits(b, 14, 2)?;
    let second = digits(b, 17, 2)?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut pos = 19;
    let mut nanos: i128 = 0;
    if b.get(pos) == Some(&b'.') {
        pos += 1;
        let start = pos;
        while pos < b.len() && b[pos].is_ascii_digit() {
            pos += 1;
        }
        let frac = &b[start..pos];
        if frac.is_empty() {
            return None;
        }
        for (i, d) in frac.iter().take(9).enumerate() {
            nanos += i128::from(d - b'0') * 10i128.pow(8 - i as u32);
        }
    }
    let offset_secs: i64 = match b.get(pos) {
        Some(b'Z' | b'z') if pos + 1 == b.len() => 0,
        Some(sign @ (b'+' | b'-')) if pos + 6 == b.len() => {
            if b[pos + 3] != b':' {
                return None;
            }
            let oh = digits(b, pos + 1, 2)?;
            let om = digits(b, pos + 4, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = i64::from(oh * 3600 + om * 60);
            if *sign == b'+' { secs } else { -secs }
        }
        _ => return None,
    };
    // A leap second (:60) is read as the last second of the minute.
    let second = second.min(59);
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + i64::from(hour * 3600 + minute * 60 + second) - offset_secs;
    Some(i128::from(secs) * 1_000_000_000 + nanos)
}

/// Parses an RFC 3339 timestamp into a `SystemTime`, or `None` when it is not one.
pub(crate) fn parse_rfc3339(input: &str) -> Option<SystemTime> {
    let nanos = parse_rfc3339_nanos(input)?;
    if nanos >= 0 {
        let n = u128::try_from(nanos).ok()?;
        UNIX_EPOCH.checked_add(Duration::new(
            u64::try_from(n / 1_000_000_000).ok()?,
            (n % 1_000_000_000) as u32,
        ))
    } else {
        let n = u128::try_from(-nanos).ok()?;
        UNIX_EPOCH.checked_sub(Duration::new(
            u64::try_from(n / 1_000_000_000).ok()?,
            (n % 1_000_000_000) as u32,
        ))
    }
}

/// Parses an optional wire timestamp.
pub(crate) fn parse_opt(value: Option<&str>) -> Option<SystemTime> {
    value.and_then(parse_rfc3339)
}

/// Formats a `SystemTime` as UTC ISO 8601 with milliseconds and `Z`
/// (`2026-11-01T00:00:00.000Z`). `None` outside the years 0000 to 9999.
pub(crate) fn format_utc_millis(time: SystemTime) -> Option<String> {
    let millis: i128 = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i128::try_from(d.as_millis()).ok()?,
        Err(e) => -i128::try_from(e.duration().as_millis()).ok()?,
    };
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = i64::try_from(secs.div_euclid(86_400)).ok()?;
    let rem = secs.rem_euclid(86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    if !(0..=9999).contains(&y) {
        return None;
    }
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    ))
}

/// Milliseconds since the Unix epoch.
pub(crate) fn epoch_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// Checks a civil date and formats it as `YYYY-MM-DD`.
pub(crate) fn format_date(year: i32, month: u32, day: u32) -> Option<String> {
    if !(0..=9999).contains(&year) || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    if day > days_in_month(i64::from(year), month) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strict_rfc3339() {
        let t = parse_rfc3339_nanos("2026-11-23T21:04:05.123Z").unwrap();
        assert_eq!(t, 1_795_467_845_123_000_000);
        assert_eq!(
            parse_rfc3339_nanos("2026-11-23T22:04:05.123+01:00"),
            Some(1_795_467_845_123_000_000)
        );
        assert_eq!(parse_rfc3339_nanos("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_rfc3339_nanos("1969-12-31T23:59:59Z"),
            Some(-1_000_000_000)
        );
        for bad in [
            "now",
            "",
            "1795467845",
            "2026-11-23T21:04:05",
            "2026-11-23 21:04:05Z",
            "2026-02-30T00:00:00Z",
            "2026-11-23T21:04:05.Z",
            "2026-11-23T24:00:00Z",
            "2026-11-23T21:04:05+0100",
            "2026-11-23T21:04:05Zjunk",
        ] {
            assert_eq!(parse_rfc3339_nanos(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_utc_millis() {
        let t = parse_rfc3339("2026-11-01T00:00:00.000Z").unwrap();
        assert_eq!(format_utc_millis(t).unwrap(), "2026-11-01T00:00:00.000Z");
        let t = parse_rfc3339("1969-07-20T20:17:40.5Z").unwrap();
        assert_eq!(format_utc_millis(t).unwrap(), "1969-07-20T20:17:40.500Z");
        assert_eq!(format_date(2026, 2, 29), None);
        assert_eq!(format_date(2024, 2, 29).unwrap(), "2024-02-29");
    }
}
