//! Proleptic Gregorian calendar dates in UTC.

use anyhow::Result;

use crate::CoreError;

/// Earliest supported calendar year.
pub(super) const MIN_YEAR: i32 = -9999;
/// Latest supported calendar year.
pub(super) const MAX_YEAR: i32 = 9999;

/// A validated UTC calendar date and time of day with millisecond precision.
///
/// Years are astronomical (year 0 exists) and limited to `-9999..=9999` so they
/// always format as four digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDateTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    millisecond: u16,
}

impl CivilDateTime {
    /// Validates and builds a date-time. Leap seconds are not representable.
    pub fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        millisecond: u16,
    ) -> Result<Self> {
        let invalid = |reason| CoreError::InvalidDateTime { reason };
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return Err(invalid("year out of range").into());
        }
        if !(1..=12).contains(&month) {
            return Err(invalid("month out of range").into());
        }
        if day == 0 || day > days_in_month(year, month) {
            return Err(invalid("day out of range for month").into());
        }
        if hour > 23 {
            return Err(invalid("hour out of range").into());
        }
        if minute > 59 {
            return Err(invalid("minute out of range").into());
        }
        if second > 59 {
            return Err(invalid("second out of range").into());
        }
        if millisecond > 999 {
            return Err(invalid("millisecond out of range").into());
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
        })
    }

    /// Builds a date-time from parts already known to be valid.
    pub(super) const fn from_parts_unchecked(
        (year, month, day): (i32, u8, u8),
        (hour, minute, second, millisecond): (u8, u8, u8, u16),
    ) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
        }
    }

    /// Astronomical year (`0` is 1 BC).
    pub const fn year(&self) -> i32 {
        self.year
    }

    /// Month, `1..=12`.
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// Day of month, `1..=31`.
    pub const fn day(&self) -> u8 {
        self.day
    }

    /// Hour, `0..=23`.
    pub const fn hour(&self) -> u8 {
        self.hour
    }

    /// Minute, `0..=59`.
    pub const fn minute(&self) -> u8 {
        self.minute
    }

    /// Second, `0..=59`.
    pub const fn second(&self) -> u8 {
        self.second
    }

    /// Millisecond, `0..=999`.
    pub const fn millisecond(&self) -> u16 {
        self.millisecond
    }
}

const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a valid date (Howard Hinnant's `days_from_civil`).
pub(super) const fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let month = month as i64;
    let year = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Date for a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
///
/// Callers keep `days` within the supported year range, so the narrowing casts
/// are lossless.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) const fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    // SAFETY: year is within MIN_YEAR..=MAX_YEAR, month in 1..=12, day in
    // 1..=31.
    (year as i32, month as u8, day as u8)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    fn reason(result: Result<CivilDateTime>) -> Result<CoreError> {
        let error = result.err().context("expected an invalid date")?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    #[test]
    fn test_civil_new_accepts_valid_leap_day() -> Result<()> {
        let date = CivilDateTime::new(2024, 2, 29, 23, 59, 59, 999)?;
        assert_eq!((date.year(), date.month(), date.day()), (2024, 2, 29));
        assert_eq!(
            (
                date.hour(),
                date.minute(),
                date.second(),
                date.millisecond()
            ),
            (23, 59, 59, 999)
        );
        Ok(())
    }

    #[test]
    fn test_civil_new_rejects_non_leap_feb_29() -> Result<()> {
        assert_eq!(
            reason(CivilDateTime::new(2100, 2, 29, 0, 0, 0, 0))?,
            CoreError::InvalidDateTime {
                reason: "day out of range for month"
            }
        );
        Ok(())
    }

    #[test]
    fn test_civil_new_rejects_out_of_range_fields() -> Result<()> {
        let cases = [
            (
                CivilDateTime::new(10_000, 1, 1, 0, 0, 0, 0),
                "year out of range",
            ),
            (
                CivilDateTime::new(-10_000, 1, 1, 0, 0, 0, 0),
                "year out of range",
            ),
            (
                CivilDateTime::new(2026, 13, 1, 0, 0, 0, 0),
                "month out of range",
            ),
            (
                CivilDateTime::new(2026, 0, 1, 0, 0, 0, 0),
                "month out of range",
            ),
            (
                CivilDateTime::new(2026, 4, 31, 0, 0, 0, 0),
                "day out of range for month",
            ),
            (
                CivilDateTime::new(2026, 4, 0, 0, 0, 0, 0),
                "day out of range for month",
            ),
            (
                CivilDateTime::new(2026, 1, 1, 24, 0, 0, 0),
                "hour out of range",
            ),
            (
                CivilDateTime::new(2026, 1, 1, 0, 60, 0, 0),
                "minute out of range",
            ),
            (
                CivilDateTime::new(2026, 1, 1, 0, 0, 60, 0),
                "second out of range",
            ),
            (
                CivilDateTime::new(2026, 1, 1, 0, 0, 0, 1000),
                "millisecond out of range",
            ),
        ];
        for (result, expected) in cases {
            assert_eq!(
                reason(result)?,
                CoreError::InvalidDateTime { reason: expected }
            );
        }
        Ok(())
    }

    #[test]
    fn test_days_from_civil_known_values() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2026, 11, 5), 20_762);
    }

    #[test]
    fn test_civil_from_days_known_values() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20_762), (2026, 11, 5));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_civil_days_round_trip(days in days_from_civil(MIN_YEAR, 1, 1)..=days_from_civil(MAX_YEAR, 12, 31)) {
            let (year, month, day) = civil_from_days(days);
            prop_assert_eq!(days_from_civil(year, month, day), days);
        }

        #[test]
        fn test_civil_from_days_is_valid_date(days in days_from_civil(MIN_YEAR, 1, 1)..=days_from_civil(MAX_YEAR, 12, 31)) {
            let (year, month, day) = civil_from_days(days);
            prop_assert!(CivilDateTime::new(year, month, day, 0, 0, 0, 0).is_ok());
        }

        #[test]
        fn test_consecutive_days_are_consecutive(days in days_from_civil(MIN_YEAR, 1, 1)..days_from_civil(MAX_YEAR, 12, 31)) {
            let (y1, m1, d1) = civil_from_days(days);
            let (y2, m2, d2) = civil_from_days(days + 1);
            let next_day = d2 == d1 + 1 && m2 == m1 && y2 == y1;
            let next_month = d2 == 1 && ((m2 == m1 + 1 && y2 == y1) || (m1 == 12 && m2 == 1 && y2 == y1 + 1));
            prop_assert!(next_day || next_month);
        }
    }
}
