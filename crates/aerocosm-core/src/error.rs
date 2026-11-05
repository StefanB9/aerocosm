//! Domain errors.

use thiserror::Error;

/// Recoverable, matchable errors raised by the core library.
///
/// Public functions return `anyhow::Result`; callers match on a variant with
/// `error.downcast_ref::<CoreError>()`.
#[derive(Debug, Error, PartialEq)]
pub enum CoreError {
    /// A table position does not fit in a 32-bit id.
    #[error("index {index} does not fit in a 32-bit id")]
    IdOverflow {
        /// The rejected table position.
        index: usize,
    },

    /// A calendar date-time component is out of range.
    #[error("invalid date-time: {reason}")]
    InvalidDateTime {
        /// Which component is invalid.
        reason: &'static str,
    },

    /// An instant lies outside the supported years -9999 to 9999.
    #[error("time {millis} ms since the Unix epoch is outside years -9999 to 9999")]
    TimeOutOfRange {
        /// The rejected millisecond count.
        millis: i64,
    },

    /// Duration arithmetic overflowed `i64` milliseconds.
    #[error("duration arithmetic overflowed")]
    DurationOverflow,

    /// Text is not an ISO 8601 UTC timestamp.
    #[error("cannot parse {input:?} as a UTC timestamp (expected YYYY-MM-DDTHH:MM:SS[.mmm]Z)")]
    ParseTime {
        /// The rejected text.
        input: String,
    },

    /// Text is not a duration like `1h30m`.
    #[error("cannot parse {input:?} as a duration (expected e.g. 24h, 1h30m, 250ms)")]
    ParseDuration {
        /// The rejected text.
        input: String,
    },

    /// A physical quantity was NaN or infinite.
    #[error("non-finite value for a quantity in {unit}")]
    NonFiniteQuantity {
        /// Unit symbol of the quantity.
        unit: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_error_id_overflow_display() {
        let error = CoreError::IdOverflow {
            index: 5_000_000_000,
        };
        assert_eq!(
            error.to_string(),
            "index 5000000000 does not fit in a 32-bit id"
        );
    }

    #[test]
    fn test_core_error_non_finite_quantity_display() {
        let error = CoreError::NonFiniteQuantity { unit: "kt" };
        assert_eq!(error.to_string(), "non-finite value for a quantity in kt");
    }
}
