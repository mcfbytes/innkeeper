//! Stored instants and civil dates. Every time the database keeps is Unix milliseconds UTC
//! (docs/server/store.md section 3); a civil date is the UTC midnight that starts it. Hold and export
//! bounds are in docs/server/privacy.md.

use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

/// One second, one minute, one hour and one day in the stored unit.
pub(crate) const SECOND: i64 = 1_000;
pub(crate) const MINUTE: i64 = 60 * SECOND;
pub(crate) const HOUR: i64 = 60 * MINUTE;
pub(crate) const DAY: i64 = 24 * HOUR;

#[derive(Debug, Error)]
pub(crate) enum DateError {
    #[error("a date is YYYY-MM-DD")]
    Format,
    #[error("the hold date {0} has already passed")]
    Past(String),
}

/// Now, in Unix milliseconds: the value every `*_at` column holds.
pub(crate) fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// Now, in Unix seconds, for the JWT `iat` and `exp` claims, which are seconds by definition.
pub(crate) fn unix_seconds() -> i64 {
    now_ms() / SECOND
}

/// Midnight UTC at the start of `text`.
pub(crate) fn parse_date(text: &str) -> Result<i64, DateError> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return Err(DateError::Format);
    }
    let year = i32::try_from(digits(bytes, 0, 4)?).map_err(|_| DateError::Format)?;
    let month = digits(bytes, 5, 2)?;
    let day = digits(bytes, 8, 2)?;
    let days = days_from_civil(year, month, day)?;
    days.checked_mul(DAY).ok_or(DateError::Format)
}

/// `YYYY-MM-DD` of the UTC day that contains the instant `at`.
pub(crate) fn format_date(at: i64) -> String {
    let (year, month, day) = civil_from_days(at.div_euclid(DAY));
    format!("{year:04}-{month:02}-{day:02}")
}

/// `YYYY-MM-DD HH:MM UTC` of the instant `at`, for the portal's tables.
pub(crate) fn format_datetime(at: i64) -> String {
    let minutes = at.div_euclid(MINUTE).rem_euclid(1440);
    format!(
        "{} {:02}:{:02} UTC",
        format_date(at),
        minutes / 60,
        minutes % 60
    )
}

/// `YYYYMMDDTHHMMSSZ` of the instant `at`, for a file name.
pub(crate) fn format_stamp(at: i64) -> String {
    let seconds = at.div_euclid(SECOND).rem_euclid(86_400);
    let date = format_date(at).replace('-', "");
    format!(
        "{date}T{:02}{:02}{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

pub(crate) fn add_days(at: i64, days: u32) -> i64 {
    at.saturating_add(i64::from(days).saturating_mul(DAY))
}

/// The hold is stored as the midnight after the named day, so that day is included.
pub(crate) fn hold_until(text: &str, now: i64) -> Result<i64, DateError> {
    let start = parse_date(text)?;
    let until = start.checked_add(DAY).ok_or(DateError::Format)?;
    if until <= now {
        return Err(DateError::Past(text.to_owned()));
    }
    Ok(until)
}

fn digits(bytes: &[u8], at: usize, len: usize) -> Result<u32, DateError> {
    let mut value = 0u32;
    for offset in 0..len {
        let byte = bytes.get(at + offset).copied().ok_or(DateError::Format)?;
        if !byte.is_ascii_digit() {
            return Err(DateError::Format);
        }
        value = value * 10 + u32::from(byte - b'0');
    }
    Ok(value)
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01.
fn days_from_civil(year: i32, month: u32, day: u32) -> Result<i64, DateError> {
    if day == 0 || day > days_in_month(year, month) {
        return Err(DateError::Format);
    }
    let year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (year, month as u32, day as u32)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unix_epoch_and_a_leap_day_round_trip() {
        assert_eq!(parse_date("1970-01-01").unwrap(), 0);
        assert_eq!(parse_date("1970-01-02").unwrap(), DAY);
        assert_eq!(parse_date("2000-01-01").unwrap(), 946_684_800_000);
        assert_eq!(format_date(946_684_800_000 + HOUR), "2000-01-01");
        assert_eq!(
            format_datetime(946_684_800_000 + 90 * MINUTE),
            "2000-01-01 01:30 UTC"
        );
        assert_eq!(
            format_stamp(946_684_800_000 + 90 * MINUTE + 7 * SECOND),
            "20000101T013007Z"
        );
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2023-02-29").is_err());
        assert!(parse_date("2026-1-16").is_err());
    }

    #[test]
    fn a_hold_includes_the_named_day_and_rejects_a_past_one() {
        let now = parse_date("2026-10-16").unwrap() + HOUR;
        let until = hold_until("2026-10-16", now).unwrap();
        assert_eq!(until, parse_date("2026-10-17").unwrap());
        assert!(matches!(
            hold_until("2026-10-15", now),
            Err(DateError::Past(_))
        ));
    }
}
