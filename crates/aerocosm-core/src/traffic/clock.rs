//! Local wall-clock times and curfew windows.

use std::{fmt, str::FromStr};

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// A local time of day with minute precision, written `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClockTime(u16);

impl ClockTime {
    /// 00:00.
    pub const MIDNIGHT: Self = Self(0);

    /// The time `minute` minutes after midnight, `0..1440`.
    pub fn from_minute(minute: u16) -> Result<Self> {
        if minute < 24 * 60 {
            Ok(Self(minute))
        } else {
            Err(CoreError::InvalidLocalTime { minute }.into())
        }
    }

    /// Minutes after midnight.
    pub const fn minute_of_day(self) -> u16 {
        self.0
    }
}

impl FromStr for ClockTime {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let invalid = || CoreError::InvalidCode {
            kind: "clock time",
            input: input.to_owned(),
        };
        let bytes = input.as_bytes();
        if bytes.len() != 5 || bytes[2] != b':' {
            return Err(invalid().into());
        }
        let field = |text: &str| -> Option<u16> {
            text.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| text.parse().ok())
                .flatten()
        };
        match (field(&input[..2]), field(&input[3..])) {
            (Some(hour), Some(minute)) if hour < 24 && minute < 60 => Ok(Self(hour * 60 + minute)),
            _ => Err(invalid().into()),
        }
    }
}

impl fmt::Display for ClockTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl Serialize for ClockTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ClockTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <&str>::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A nightly window without scheduled movements: from `start` (inclusive) to
/// `end` (exclusive), wrapping past midnight when `start > end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curfew {
    /// First closed minute.
    pub start: ClockTime,
    /// First open minute again.
    pub end: ClockTime,
}

impl Curfew {
    /// Whether `time` falls inside the window.
    pub fn contains(self, time: ClockTime) -> bool {
        if self.start <= self.end {
            self.start <= time && time < self.end
        } else {
            time >= self.start || time < self.end
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    #[test]
    fn test_clock_time_parse_and_display() -> Result<()> {
        let time: ClockTime = "06:30".parse()?;
        assert_eq!(time.minute_of_day(), 390);
        assert_eq!(time.to_string(), "06:30");
        assert_eq!("00:00".parse::<ClockTime>()?.minute_of_day(), 0);
        assert_eq!("23:59".parse::<ClockTime>()?.minute_of_day(), 1439);
        for bad in ["24:00", "6:30", "06:60", "0630", "06:3", ""] {
            let error = bad
                .parse::<ClockTime>()
                .err()
                .with_context(|| format!("{bad:?}"))?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::InvalidCode {
                    kind: "clock time",
                    input: bad.to_owned()
                })
            );
        }
        Ok(())
    }

    #[test]
    fn test_clock_time_serde_as_string() -> Result<()> {
        let time: ClockTime = "21:15".parse()?;
        assert_eq!(
            serde_json::to_string(&time).context("serialise")?,
            "\"21:15\""
        );
        assert!(serde_json::from_str::<ClockTime>("\"25:00\"").is_err());
        Ok(())
    }

    #[test]
    fn test_curfew_across_midnight() -> Result<()> {
        let curfew = Curfew {
            start: "23:00".parse()?,
            end: "05:00".parse()?,
        };
        assert!(curfew.contains("23:00".parse()?));
        assert!(curfew.contains("02:00".parse()?));
        assert!(
            !curfew.contains("05:00".parse()?),
            "the end minute is open again"
        );
        assert!(!curfew.contains("12:00".parse()?));
        Ok(())
    }

    #[test]
    fn test_curfew_within_one_day() -> Result<()> {
        let curfew = Curfew {
            start: "00:00".parse()?,
            end: "05:00".parse()?,
        };
        assert!(curfew.contains("04:59".parse()?));
        assert!(!curfew.contains("23:30".parse()?));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_clock_time_round_trip(minute in 0_u16..1440) {
            let time = ClockTime::from_minute(minute).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(time.to_string().parse::<ClockTime>().ok(), Some(time));
        }

        #[test]
        fn test_curfew_length_matches_window(start in 0_u16..1440, end in 0_u16..1440) {
            prop_assume!(start != end);
            let curfew = Curfew {
                start: ClockTime::from_minute(start).map_err(|e| TestCaseError::fail(e.to_string()))?,
                end: ClockTime::from_minute(end).map_err(|e| TestCaseError::fail(e.to_string()))?,
            };
            let closed = (0..1440).filter(|m| ClockTime::from_minute(*m).is_ok_and(|t| curfew.contains(t))).count();
            let expected = usize::from((end + 1440 - start) % 1440);
            prop_assert_eq!(closed, expected);
        }
    }
}
