//! Simulated time: instants, durations and calendar conversion.
//!
//! All simulated time is integer milliseconds in UTC so arithmetic is exact and
//! identical on every platform, which the deterministic simulation depends on.

mod civil;
mod duration;
mod zone;

use std::{fmt, str::FromStr};

use anyhow::Result;
pub use civil::CivilDateTime;
use civil::{MAX_YEAR, MIN_YEAR, civil_from_days, days_from_civil};
pub use duration::SimDuration;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
pub use zone::{LocalDate, LocalTime, Weekday, Zone};

use crate::CoreError;

const MILLIS_PER_DAY: i64 = 86_400_000;

/// An instant of simulated time: UTC milliseconds since 1970-01-01T00:00:00Z.
///
/// Always within [`SimTime::MIN`]..=[`SimTime::MAX`] (years -9999 to 9999), so
/// calendar conversion is infallible and any two instants differ by a
/// representable duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimTime(i64);

impl SimTime {
    /// 9999-12-31T23:59:59.999Z.
    pub const MAX: Self = Self((days_from_civil(MAX_YEAR, 12, 31) + 1) * MILLIS_PER_DAY - 1);
    /// -9999-01-01T00:00:00.000Z.
    pub const MIN: Self = Self(days_from_civil(MIN_YEAR, 1, 1) * MILLIS_PER_DAY);
    /// 1970-01-01T00:00:00.000Z.
    pub const UNIX_EPOCH: Self = Self(0);

    /// The instant `millis` after the Unix epoch, failing outside the supported
    /// range.
    pub fn from_millis(millis: i64) -> Result<Self> {
        if (Self::MIN.0..=Self::MAX.0).contains(&millis) {
            Ok(Self(millis))
        } else {
            Err(CoreError::TimeOutOfRange { millis }.into())
        }
    }

    /// Milliseconds since the Unix epoch.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// The instant for a calendar date-time.
    pub const fn from_civil(civil: CivilDateTime) -> Self {
        let days = days_from_civil(civil.year(), civil.month(), civil.day());
        let millis_of_day = civil.hour() as i64 * 3_600_000
            + civil.minute() as i64 * 60_000
            + civil.second() as i64 * 1000
            + civil.millisecond() as i64;
        Self(days * MILLIS_PER_DAY + millis_of_day)
    }

    /// The calendar date-time of this instant.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub const fn to_civil(self) -> CivilDateTime {
        let days = self.0.div_euclid(MILLIS_PER_DAY);
        let ms = self.0.rem_euclid(MILLIS_PER_DAY);
        // SAFETY: `ms` is in 0..86_400_000, so each component fits its narrow
        // type.
        let time = (
            (ms / 3_600_000) as u8,
            (ms / 60_000 % 60) as u8,
            (ms / 1000 % 60) as u8,
            (ms % 1000) as u16,
        );
        CivilDateTime::from_parts_unchecked(civil_from_days(days), time)
    }

    /// This instant moved forward by `duration`, failing outside the supported
    /// range.
    pub fn checked_add(self, duration: SimDuration) -> Result<Self> {
        let millis = self
            .0
            .checked_add(duration.as_millis())
            .ok_or(CoreError::DurationOverflow)?;
        Self::from_millis(millis)
    }

    /// This instant moved back by `duration`, failing outside the supported
    /// range.
    pub fn checked_sub(self, duration: SimDuration) -> Result<Self> {
        let millis = self
            .0
            .checked_sub(duration.as_millis())
            .ok_or(CoreError::DurationOverflow)?;
        Self::from_millis(millis)
    }

    /// Signed span from `earlier` to `self`. Cannot overflow because both are
    /// in range.
    pub const fn duration_since(self, earlier: Self) -> SimDuration {
        SimDuration::from_millis(self.0 - earlier.0)
    }
}

impl fmt::Display for SimTime {
    /// ISO 8601 with millisecond precision, e.g. `2026-11-05T06:00:00.000Z`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.to_civil();
        let sign = if c.year() < 0 { "-" } else { "" };
        write!(
            f,
            "{sign}{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            c.year().unsigned_abs(),
            c.month(),
            c.day(),
            c.hour(),
            c.minute(),
            c.second(),
            c.millisecond()
        )
    }
}

impl FromStr for SimTime {
    type Err = anyhow::Error;

    /// Parses `[-]YYYY-MM-DDTHH:MM:SS[.mmm]Z`, the format produced by
    /// `Display`.
    fn from_str(input: &str) -> Result<Self> {
        parse_iso(input).map(Self::from_civil).ok_or_else(|| {
            CoreError::ParseTime {
                input: input.to_owned(),
            }
            .into()
        })
    }
}

fn parse_iso(input: &str) -> Option<CivilDateTime> {
    let (negative, body) = match input.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, input),
    };
    let body = body.strip_suffix('Z')?;
    let (date, time) = body.split_once('T')?;
    let (time, millis) = match time.split_once('.') {
        Some((hms, ms)) if ms.len() == 3 => (hms, fixed_digits(ms)?),
        Some(_) => return None,
        None => (time, 0),
    };
    let mut date_parts = date.split('-');
    let year = i32::try_from(fixed_digits(date_parts.next().filter(|y| y.len() == 4)?)?).ok()?;
    let month = u8::try_from(fixed_digits(date_parts.next().filter(|m| m.len() == 2)?)?).ok()?;
    let day = u8::try_from(fixed_digits(date_parts.next().filter(|d| d.len() == 2)?)?).ok()?;
    let mut time_parts = time.split(':');
    let mut two = || u8::try_from(fixed_digits(time_parts.next().filter(|p| p.len() == 2)?)?).ok();
    let (hour, minute, second) = (two()?, two()?, two()?);
    if date_parts.next().is_some() || time_parts.next().is_some() {
        return None;
    }
    let year = if negative { -year } else { year };
    CivilDateTime::new(
        year,
        month,
        day,
        hour,
        minute,
        second,
        u16::try_from(millis).ok()?,
    )
    .ok()
}

/// Parses an all-ASCII-digit string (no sign, no whitespace).
fn fixed_digits(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl Serialize for SimTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SimTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    fn in_range_millis() -> impl Strategy<Value = i64> {
        SimTime::MIN.as_millis()..=SimTime::MAX.as_millis()
    }

    #[test]
    fn test_sim_time_unix_epoch_is_zero() {
        assert_eq!(SimTime::UNIX_EPOCH.as_millis(), 0);
    }

    #[test]
    fn test_sim_time_from_millis_rejects_out_of_range() -> Result<()> {
        let too_late = SimTime::MAX.as_millis() + 1;
        let error = SimTime::from_millis(too_late)
            .err()
            .context("beyond year 9999 must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::TimeOutOfRange { millis: too_late })
        );
        assert!(SimTime::from_millis(SimTime::MIN.as_millis() - 1).is_err());
        Ok(())
    }

    #[test]
    fn test_sim_time_bounds_are_year_limits() {
        let min = SimTime::MIN.to_civil();
        let max = SimTime::MAX.to_civil();
        assert_eq!(
            (min.year(), min.month(), min.day(), min.hour()),
            (-9999, 1, 1, 0)
        );
        assert_eq!(
            (max.year(), max.month(), max.day(), max.millisecond()),
            (9999, 12, 31, 999)
        );
    }

    #[test]
    fn test_sim_time_from_civil_known_value() -> Result<()> {
        let civil = CivilDateTime::new(2026, 11, 5, 13, 41, 18, 250)?;
        let time = SimTime::from_civil(civil);
        assert_eq!(time.as_millis(), 1_793_886_078_250);
        assert_eq!(time.to_civil(), civil);
        Ok(())
    }

    #[test]
    fn test_sim_time_display_iso_8601() -> Result<()> {
        let time = SimTime::from_civil(CivilDateTime::new(2026, 1, 2, 3, 4, 5, 6)?);
        assert_eq!(time.to_string(), "2026-01-02T03:04:05.006Z");
        let early = SimTime::from_civil(CivilDateTime::new(-44, 3, 15, 0, 0, 0, 0)?);
        assert_eq!(early.to_string(), "-0044-03-15T00:00:00.000Z");
        Ok(())
    }

    #[test]
    fn test_sim_time_parse_with_and_without_millis() -> Result<()> {
        let expected = SimTime::from_civil(CivilDateTime::new(2026, 11, 5, 6, 0, 0, 0)?);
        assert_eq!("2026-11-05T06:00:00Z".parse::<SimTime>()?, expected);
        assert_eq!("2026-11-05T06:00:00.000Z".parse::<SimTime>()?, expected);
        Ok(())
    }

    #[test]
    fn test_sim_time_parse_rejects_malformed() -> Result<()> {
        for input in [
            "",
            "2026-11-05",
            "2026-11-05T06:00:00",
            "2026-11-05 06:00:00Z",
            "2026-13-05T06:00:00Z",
            "2026-11-05T06:00:00.5Z",
            "2026-11-05T06:00:00+01:00",
        ] {
            let error = input
                .parse::<SimTime>()
                .err()
                .with_context(|| format!("{input:?} must be rejected"))?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::ParseTime {
                    input: input.to_owned()
                })
            );
        }
        Ok(())
    }

    #[test]
    fn test_sim_time_checked_add_rejects_overflow() {
        assert!(
            SimTime::MAX
                .checked_add(SimDuration::from_millis(1))
                .is_err()
        );
        assert!(
            SimTime::MIN
                .checked_sub(SimDuration::from_millis(1))
                .is_err()
        );
    }

    #[test]
    fn test_sim_time_serde_is_iso_string() -> Result<()> {
        let time = "2026-11-05T06:00:00.000Z".parse::<SimTime>()?;
        let json = serde_json::to_string(&time).context("serialise")?;
        assert_eq!(json, "\"2026-11-05T06:00:00.000Z\"");
        let back: SimTime = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back, time);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_sim_time_civil_round_trip(ms in in_range_millis()) {
            let time = SimTime::from_millis(ms).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(SimTime::from_civil(time.to_civil()), time);
        }

        #[test]
        fn test_sim_time_display_parse_round_trip(ms in in_range_millis()) {
            let time = SimTime::from_millis(ms).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(time.to_string().parse::<SimTime>().ok(), Some(time));
        }

        #[test]
        fn test_sim_time_add_then_since_recovers_duration(
            start in -1_i64 << 40..1_i64 << 40,
            delta in -1_i64 << 40..1_i64 << 40,
        ) {
            let start = SimTime::from_millis(start).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let delta = SimDuration::from_millis(delta);
            let end = start.checked_add(delta).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(end.duration_since(start), delta);
            prop_assert_eq!(end.checked_sub(delta).ok(), Some(start));
        }

        #[test]
        fn test_sim_time_ordering_matches_millis(a in in_range_millis(), b in in_range_millis()) {
            let ta = SimTime::from_millis(a).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let tb = SimTime::from_millis(b).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(ta.cmp(&tb), a.cmp(&b));
        }
    }
}
