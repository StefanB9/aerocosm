//! `aerocosm bench <scenario>` (plan M5 D135, D138): fixed world scenarios
//! timed in their own process, each result printed as one line for
//! `cargo xtask bench` to record.

pub(crate) mod memory;
pub(crate) mod stats;
pub(crate) mod world;

use std::fmt;

/// A measured value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Value {
    /// A quantity in `unit`.
    Number(f64, Unit),
    /// A fingerprint (plan M5 D137).
    Hash(u64),
}

/// The unit of a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Milliseconds,
    Seconds,
    MiB,
    Count,
    PerSecond,
}

impl Unit {
    const fn symbol(self) -> &'static str {
        match self {
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::MiB => "MiB",
            Self::Count => "count",
            Self::PerSecond => "1/s",
        }
    }
}

/// One named result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measure {
    pub(crate) name: String,
    pub(crate) value: Value,
}

impl Measure {
    pub(crate) fn number(name: impl Into<String>, value: f64, unit: Unit) -> Self {
        Self {
            name: name.into(),
            value: Value::Number(value, unit),
        }
    }
}

/// Lines start with this word, so that other output can be told apart.
pub(crate) const LINE_PREFIX: &str = "bench";

impl fmt::Display for Measure {
    /// `bench <name> <value> <unit>`: numbers with up to six decimals,
    /// hashes as 16 hexadecimal digits with the unit `hash`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{LINE_PREFIX} {} ", self.name)?;
        match self.value {
            Value::Hash(hash) => write!(f, "{hash:016x} hash"),
            Value::Number(number, unit) => {
                let text = format!("{number:.6}");
                let text = text.trim_end_matches('0').trim_end_matches('.');
                write!(f, "{text} {}", unit.symbol())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measures_print_as_lines() {
        assert_eq!(
            Measure::number("tick-12/median", 10.25, Unit::Milliseconds).to_string(),
            "bench tick-12/median 10.25 ms"
        );
        assert_eq!(
            Measure::number("day/flights", 98_765.0, Unit::Count).to_string(),
            "bench day/flights 98765 count"
        );
        assert_eq!(
            Measure::number("startup/memory_peak", 1.0 / 3.0, Unit::MiB).to_string(),
            "bench startup/memory_peak 0.333333 MiB"
        );
        assert_eq!(
            Measure::number("day/wall", 2.5, Unit::Seconds).to_string(),
            "bench day/wall 2.5 s"
        );
        assert_eq!(
            Measure::number("day/ticks_per_second", 80.0, Unit::PerSecond).to_string(),
            "bench day/ticks_per_second 80 1/s"
        );
        let hash = Measure {
            name: "day/fingerprint".to_owned(),
            value: Value::Hash(0xab),
        };
        assert_eq!(
            hash.to_string(),
            "bench day/fingerprint 00000000000000ab hash"
        );
    }
}
