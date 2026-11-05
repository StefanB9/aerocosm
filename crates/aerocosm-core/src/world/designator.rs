//! Runway end designators such as `07L`.

use std::{fmt, str::FromStr};

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// Which of several parallel runways a designator names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RunwaySide {
    /// `L`.
    Left,
    /// `C`.
    Center,
    /// `R`.
    Right,
}

/// A runway end designator such as `07L`: the magnetic heading in tens of
/// degrees (`01`–`36`) and an optional parallel-runway side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunwayDesignator {
    number: u8,
    side: Option<RunwaySide>,
}

impl RunwayDesignator {
    /// Magnetic heading in tens of degrees, `1..=36`.
    pub const fn number(self) -> u8 {
        self.number
    }

    /// Parallel-runway side, if any.
    pub const fn side(self) -> Option<RunwaySide> {
        self.side
    }

    /// The designator of the opposite end: `07L` ↔ `25R`.
    #[must_use]
    pub const fn reciprocal(self) -> Self {
        let number = if self.number > 18 {
            self.number - 18
        } else {
            self.number + 18
        };
        let side = match self.side {
            Some(RunwaySide::Left) => Some(RunwaySide::Right),
            Some(RunwaySide::Right) => Some(RunwaySide::Left),
            other => other,
        };
        Self { number, side }
    }
}

impl FromStr for RunwayDesignator {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        parse_designator(input).ok_or_else(|| {
            CoreError::InvalidCode {
                kind: "runway designator",
                input: input.to_owned(),
            }
            .into()
        })
    }
}

fn parse_designator(input: &str) -> Option<RunwayDesignator> {
    let bytes = input.as_bytes();
    let (digits, suffix) = (bytes.get(..2)?, bytes.get(2..)?);
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let number = (digits[0] - b'0') * 10 + (digits[1] - b'0');
    if !(1..=36).contains(&number) {
        return None;
    }
    let side = match suffix {
        [] => None,
        [b'L'] => Some(RunwaySide::Left),
        [b'C'] => Some(RunwaySide::Center),
        [b'R'] => Some(RunwaySide::Right),
        _ => return None,
    };
    Some(RunwayDesignator { number, side })
}

impl fmt::Display for RunwayDesignator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = match self.side {
            None => "",
            Some(RunwaySide::Left) => "L",
            Some(RunwaySide::Center) => "C",
            Some(RunwaySide::Right) => "R",
        };
        write!(f, "{:02}{side}", self.number)
    }
}

impl Serialize for RunwayDesignator {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RunwayDesignator {
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

    #[test]
    fn test_designator_parse_and_display() -> Result<()> {
        let designator: RunwayDesignator = "07L".parse()?;
        assert_eq!(designator.number(), 7);
        assert_eq!(designator.side(), Some(RunwaySide::Left));
        assert_eq!(designator.to_string(), "07L");
        assert_eq!("36".parse::<RunwayDesignator>()?.to_string(), "36");
        assert_eq!(
            "09C".parse::<RunwayDesignator>()?.side(),
            Some(RunwaySide::Center)
        );
        Ok(())
    }

    #[test]
    fn test_designator_rejects_malformed() -> Result<()> {
        for input in ["", "7", "00", "37", "07X", "07LL", "H-NORTH", "7L", "07l"] {
            let error = input
                .parse::<RunwayDesignator>()
                .err()
                .with_context(|| format!("{input:?}"))?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::InvalidCode {
                    kind: "runway designator",
                    input: input.to_owned()
                })
            );
        }
        Ok(())
    }

    #[test]
    fn test_designator_reciprocal() -> Result<()> {
        for (a, b) in [
            ("07L", "25R"),
            ("25R", "07L"),
            ("18", "36"),
            ("36", "18"),
            ("09C", "27C"),
            ("01", "19"),
        ] {
            assert_eq!(
                a.parse::<RunwayDesignator>()?.reciprocal(),
                b.parse::<RunwayDesignator>()?
            );
        }
        Ok(())
    }

    #[test]
    fn test_designator_serde_as_string() -> Result<()> {
        let designator: RunwayDesignator = "25R".parse()?;
        assert_eq!(
            serde_json::to_string(&designator).context("serialise")?,
            "\"25R\""
        );
        assert!(serde_json::from_str::<RunwayDesignator>("\"40\"").is_err());
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_designator_round_trip_and_double_reciprocal(number in 1_u8..=36, side in 0_usize..4) {
            let suffix = ["", "L", "C", "R"][side];
            let text = format!("{number:02}{suffix}");
            let designator: RunwayDesignator = text.parse().map_err(|e: anyhow::Error| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(designator.to_string(), text);
            prop_assert_eq!(designator.reciprocal().reciprocal(), designator);
            let opposite = i16::from(designator.reciprocal().number());
            prop_assert_eq!((i16::from(number) - opposite).abs(), 18);
        }
    }
}
