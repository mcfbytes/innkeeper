//! The host's wall-clock time, provided to clients in reply to HostInfoRequest::HostTime.
//! Layouts in docs/protocol/int14h-census/{redbaron,golf}.md sections 8.2 and 6.2.

use std::time::{SystemTime, UNIX_EPOCH};

/// A point in time as six bytes: year-1900, month 0-based, day, hour, minute, second.
/// The host sends these to clients so the sim date and sun agree with each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostTime {
    pub year_1900: u8,
    pub month0: u8,
    pub mday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// A clock injected into the World; its implementation varies (system, fixed, mock).
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The current moment in civil time.
    fn now(&self) -> HostTime;
}

/// The system clock: uses the wall clock via std::time::SystemTime.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> HostTime {
        // Safety: unwrap is safe; system time is post-1970 on modern systems.
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let total_seconds = duration.as_secs();

        // Break Unix epoch seconds into year/month/day/hour/minute/second.
        let (year, remaining_secs) = calc_year(total_seconds);
        let (month, day, secs_in_day) = calc_month_day(year, remaining_secs);
        let (hour, minute, second) = calc_time(secs_in_day);

        // Adjust year to years since 1900.
        let year_1900 = (year - 1900) as u8;

        HostTime {
            year_1900,
            month0: month,
            mday: day,
            hour,
            minute,
            second,
        }
    }
}

/// A fixed clock for testing: always returns the same time.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    pub time: HostTime,
}

impl Clock for FixedClock {
    fn now(&self) -> HostTime {
        self.time
    }
}

/// Calculate year and remaining seconds from Unix timestamp.
/// Returns (year, remaining_secs in that year).
fn calc_year(total_secs: u64) -> (u32, u64) {
    let mut year = 1970u32;
    let mut remaining = total_secs;

    loop {
        let year_secs = if is_leap_year(year) {
            366 * 86400
        } else {
            365 * 86400
        };
        if remaining >= year_secs {
            remaining -= year_secs;
            year += 1;
        } else {
            break;
        }
    }
    (year, remaining)
}

/// Is this a leap year? Gregorian calendar rules.
fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Calculate month, day, and seconds-in-day from remaining seconds in the year.
/// Returns (month 0-based, day 1-based, secs_in_day).
fn calc_month_day(year: u32, remaining_secs: u64) -> (u8, u8, u64) {
    let secs_in_day = 86400u64;
    let day_of_year = (remaining_secs / secs_in_day) as u32;
    let secs_in_day_result = remaining_secs % secs_in_day;

    let days_in_month = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut day_count = 0;
    let mut month = 0u8;
    for (i, &days) in days_in_month.iter().enumerate() {
        if day_count + days as u32 > day_of_year {
            month = i as u8;
            break;
        }
        day_count += days as u32;
    }
    let mday = (day_of_year - day_count) as u8 + 1;

    (month, mday, secs_in_day_result)
}

/// Calculate hour, minute, second from seconds in day.
fn calc_time(secs_in_day: u64) -> (u8, u8, u8) {
    let hour = ((secs_in_day / 3600) % 24) as u8;
    let minute = ((secs_in_day / 60) % 60) as u8;
    let second = (secs_in_day % 60) as u8;
    (hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_the_set_time() {
        let time = HostTime {
            year_1900: 94,
            month0: 1,
            mday: 15,
            hour: 10,
            minute: 30,
            second: 45,
        };
        let clock = FixedClock { time };
        assert_eq!(clock.now(), time);
    }

    #[test]
    fn leap_year_detection() {
        assert!(is_leap_year(2000)); // divisible by 400
        assert!(!is_leap_year(1900)); // divisible by 100 but not 400
        assert!(is_leap_year(2004)); // divisible by 4 but not 100
        assert!(!is_leap_year(2001)); // not divisible by 4
    }

    #[test]
    fn system_clock_produces_reasonable_values() {
        let clock = SystemClock;
        let time = clock.now();
        // The year should be 1900 + something reasonable (1970-2100)
        assert!(time.year_1900 >= 70 && time.year_1900 <= 200);
        // Month should be 0-11
        assert!(time.month0 < 12);
        // Day should be 1-31
        assert!(time.mday >= 1 && time.mday <= 31);
        // Hour should be 0-23
        assert!(time.hour < 24);
        // Minute should be 0-59
        assert!(time.minute < 60);
        // Second should be 0-59
        assert!(time.second < 60);
    }

    #[test]
    fn year_and_month_boundary() {
        // Test around a year boundary (e.g., Dec 31 -> Jan 1).
        // Epoch + 365 days = Jan 1, 1971
        let epoch_plus_one_year = 365 * 86400u64;
        let (year, remaining) = calc_year(epoch_plus_one_year);
        assert_eq!(year, 1971);
        assert_eq!(remaining, 0);
    }
}
