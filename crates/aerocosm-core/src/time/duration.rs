//! Signed spans of simulated time.

use std::{fmt, str::FromStr};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::CoreError;

const MILLIS_PER_SECOND: i64 = 1000;
const MILLIS_PER_MINUTE: i64 = 60 * MILLIS_PER_SECOND;
const MILLIS_PER_HOUR: i64 = 60 * MILLIS_PER_MINUTE;
const MILLIS_PER_DAY: i64 = 24 * MILLIS_PER_HOUR;

/// A signed span of simulated time in whole milliseconds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SimDuration(i64);

impl SimDuration {
    /// The empty duration.
    pub const ZERO: Self = Self(0);

    /// A duration of `millis` milliseconds.
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// A duration of `secs` seconds. The `i32` input cannot overflow.
    pub const fn from_secs(secs: i32) -> Self {
        Self(secs as i64 * MILLIS_PER_SECOND)
    }

    /// A duration of `minutes` minutes. The `i32` input cannot overflow.
    pub const fn from_minutes(minutes: i32) -> Self {
        Self(minutes as i64 * MILLIS_PER_MINUTE)
    }

    /// A duration of `hours` hours. The `i32` input cannot overflow.
    pub const fn from_hours(hours: i32) -> Self {
        Self(hours as i64 * MILLIS_PER_HOUR)
    }

    /// Whole milliseconds.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Seconds as a float, for display and approximate physics.
    #[allow(clippy::cast_precision_loss)]
    pub const fn as_secs_f64(self) -> f64 {
        // Exact for spans below 2^53 ms (~285k years).
        self.0 as f64 / 1000.0
    }

    /// Sum of two durations, failing on `i64` overflow.
    pub fn checked_add(self, other: Self) -> Result<Self> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or_else(|| CoreError::DurationOverflow.into())
    }
}

impl fmt::Display for SimDuration {
    /// Formats as `[-]HH:MM:SS.mmm`; hours are not wrapped at 24.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let total = self.0.unsigned_abs();
        let hours = total / MILLIS_PER_HOUR.unsigned_abs();
        let minutes = total / MILLIS_PER_MINUTE.unsigned_abs() % 60;
        let seconds = total / MILLIS_PER_SECOND.unsigned_abs() % 60;
        let millis = total % MILLIS_PER_SECOND.unsigned_abs();
        write!(f, "{sign}{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
    }
}

impl FromStr for SimDuration {
    type Err = anyhow::Error;

    /// Parses a non-negative sequence of `<digits><unit>` terms, e.g. `1h30m`
    /// or `250ms`. Units: `d`, `h`, `m`, `s`, `ms`.
    fn from_str(input: &str) -> Result<Self> {
        parse_terms(input).ok_or_else(|| {
            CoreError::ParseDuration {
                input: input.to_owned(),
            }
            .into()
        })
    }
}

fn parse_terms(input: &str) -> Option<SimDuration> {
    if input.is_empty() {
        return None;
    }
    let mut rest = input;
    let mut total: i64 = 0;
    while !rest.is_empty() {
        let digits_end = rest.find(|c: char| !c.is_ascii_digit())?;
        if digits_end == 0 {
            return None;
        }
        let value: i64 = rest[..digits_end].parse().ok()?;
        rest = &rest[digits_end..];
        let (unit_millis, unit_len) = if rest.starts_with("ms") {
            (1, 2)
        } else {
            let unit = match rest.as_bytes().first()? {
                b'd' => MILLIS_PER_DAY,
                b'h' => MILLIS_PER_HOUR,
                b'm' => MILLIS_PER_MINUTE,
                b's' => MILLIS_PER_SECOND,
                _ => return None,
            };
            (unit, 1)
        };
        rest = &rest[unit_len..];
        total = total.checked_add(value.checked_mul(unit_millis)?)?;
    }
    Some(SimDuration(total))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    #[test]
    fn test_duration_constructors_agree() {
        assert_eq!(SimDuration::from_secs(90).as_millis(), 90_000);
        assert_eq!(SimDuration::from_minutes(2), SimDuration::from_secs(120));
        assert_eq!(SimDuration::from_hours(1), SimDuration::from_minutes(60));
        assert_eq!(SimDuration::from_millis(-5).as_millis(), -5);
        assert_eq!(SimDuration::ZERO.as_millis(), 0);
    }

    #[test]
    fn test_duration_as_secs_f64() {
        assert!((SimDuration::from_millis(1500).as_secs_f64() - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_duration_checked_add_detects_overflow() -> Result<()> {
        let sum = SimDuration::from_secs(1).checked_add(SimDuration::from_secs(2))?;
        assert_eq!(sum, SimDuration::from_secs(3));
        let error = SimDuration::from_millis(i64::MAX)
            .checked_add(SimDuration::from_millis(1))
            .err()
            .context("overflow must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::DurationOverflow)
        );
        Ok(())
    }

    #[test]
    fn test_duration_display() {
        assert_eq!(
            SimDuration::from_millis(3_723_004).to_string(),
            "01:02:03.004"
        );
        assert_eq!(SimDuration::from_hours(26).to_string(), "26:00:00.000");
        assert_eq!(SimDuration::from_millis(-1500).to_string(), "-00:00:01.500");
    }

    #[test]
    fn test_duration_parse_units() -> Result<()> {
        assert_eq!("24h".parse::<SimDuration>()?, SimDuration::from_hours(24));
        assert_eq!(
            "1h30m".parse::<SimDuration>()?,
            SimDuration::from_minutes(90)
        );
        assert_eq!("2d".parse::<SimDuration>()?, SimDuration::from_hours(48));
        assert_eq!("45s".parse::<SimDuration>()?, SimDuration::from_secs(45));
        assert_eq!(
            "250ms".parse::<SimDuration>()?,
            SimDuration::from_millis(250)
        );
        assert_eq!(
            "1m500ms".parse::<SimDuration>()?,
            SimDuration::from_millis(60_500)
        );
        Ok(())
    }

    #[test]
    fn test_duration_parse_rejects_malformed() -> Result<()> {
        for input in [
            "",
            "h",
            "10",
            "10x",
            "1h-5m",
            "99999999999999999999d",
            " 5s",
        ] {
            let error = input
                .parse::<SimDuration>()
                .err()
                .with_context(|| format!("{input:?} must be rejected"))?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::ParseDuration {
                    input: input.to_owned()
                })
            );
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_duration_add_is_commutative(a in -1_i64 << 50..1_i64 << 50, b in -1_i64 << 50..1_i64 << 50) {
            let (a, b) = (SimDuration::from_millis(a), SimDuration::from_millis(b));
            prop_assert_eq!(a.checked_add(b).ok(), b.checked_add(a).ok());
        }

        #[test]
        fn test_duration_parse_millis_round_trip(ms in 0_i64..1_i64 << 50) {
            let parsed = format!("{ms}ms").parse::<SimDuration>();
            prop_assert_eq!(parsed.ok(), Some(SimDuration::from_millis(ms)));
        }
    }
}
