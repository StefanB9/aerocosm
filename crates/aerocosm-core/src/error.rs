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

    /// A latitude or longitude is out of range or not finite.
    #[error("invalid coordinate lat={lat} lon={lon} (expected -90..=90, -180..=180)")]
    InvalidCoordinate {
        /// Rejected latitude in degrees.
        lat: f64,
        /// Rejected longitude in degrees.
        lon: f64,
    },

    /// An interpolation fraction is outside `0..=1`.
    #[error("interpolation fraction {fraction} is outside 0..=1")]
    InvalidFraction {
        /// The rejected fraction.
        fraction: f64,
    },

    /// Two points are antipodal, so the great circle between them is not
    /// unique.
    #[error("points are antipodal; the great circle between them is ambiguous")]
    AntipodalPoints,

    /// A random draw was requested from an empty range.
    #[error("cannot draw from an empty range")]
    EmptyRange,
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

    #[test]
    fn test_core_error_invalid_coordinate_display() {
        let error = CoreError::InvalidCoordinate {
            lat: 91.0,
            lon: 0.5,
        };
        assert_eq!(
            error.to_string(),
            "invalid coordinate lat=91 lon=0.5 (expected -90..=90, -180..=180)"
        );
    }

    #[test]
    fn test_core_error_invalid_fraction_display() {
        let error = CoreError::InvalidFraction { fraction: 1.5 };
        assert_eq!(
            error.to_string(),
            "interpolation fraction 1.5 is outside 0..=1"
        );
    }

    #[test]
    fn test_core_error_antipodal_points_display() {
        assert_eq!(
            CoreError::AntipodalPoints.to_string(),
            "points are antipodal; the great circle between them is ambiguous"
        );
    }

    #[test]
    fn test_core_error_empty_range_display() {
        assert_eq!(
            CoreError::EmptyRange.to_string(),
            "cannot draw from an empty range"
        );
    }
}
