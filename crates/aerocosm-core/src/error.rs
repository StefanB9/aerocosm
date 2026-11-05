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

    /// A pacer was configured with a non-positive tick or frame budget.
    #[error("invalid pacer configuration: {reason}")]
    InvalidPacerConfig {
        /// What is wrong with the configuration.
        reason: &'static str,
    },

    /// An aviation code or identifier is malformed.
    #[error("invalid {kind}: {input:?}")]
    InvalidCode {
        /// What kind of code was expected.
        kind: &'static str,
        /// The rejected text.
        input: String,
    },

    /// An airport definition is inconsistent.
    #[error("invalid airport {ident}: {reason}")]
    InvalidAirport {
        /// Ident of the rejected airport.
        ident: String,
        /// What is wrong.
        reason: &'static str,
    },

    /// A runway end is inconsistent.
    #[error("invalid runway end {designator}: {reason}")]
    InvalidRunwayEnd {
        /// Designator of the rejected end.
        designator: String,
        /// What is wrong.
        reason: &'static str,
    },

    /// A runway definition is inconsistent.
    #[error("invalid runway {designator} at {airport}: {reason}")]
    InvalidRunway {
        /// Ident of the runway's airport.
        airport: String,
        /// Runway name, e.g. `07C/25C`.
        designator: String,
        /// What is wrong.
        reason: &'static str,
    },

    /// Two airports share an ident.
    #[error("duplicate airport ident {ident}")]
    DuplicateAirport {
        /// The repeated ident.
        ident: String,
    },

    /// A runway references an airport that does not exist.
    #[error("runway references unknown airport {ident}")]
    UnknownAirport {
        /// The unresolved ident.
        ident: String,
    },

    /// A data file has a format version this build cannot read.
    #[error("{file} format {found} is not supported (expected {supported})")]
    UnsupportedFileFormat {
        /// Which kind of file.
        file: &'static str,
        /// Version found in the file.
        found: u32,
        /// Version this build reads.
        supported: u32,
    },

    /// An aircraft type definition is implausible.
    #[error("invalid aircraft type {code}: {reason}")]
    InvalidAircraftType {
        /// Designator of the rejected type.
        code: String,
        /// What is wrong.
        reason: &'static str,
    },

    /// Two aircraft types share a designator.
    #[error("duplicate aircraft type {code}")]
    DuplicateAircraftType {
        /// The repeated designator.
        code: String,
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

    #[test]
    fn test_core_error_invalid_pacer_config_display() {
        let error = CoreError::InvalidPacerConfig {
            reason: "tick must be positive",
        };
        assert_eq!(
            error.to_string(),
            "invalid pacer configuration: tick must be positive"
        );
    }

    #[test]
    fn test_core_error_invalid_code_display() {
        let error = CoreError::InvalidCode {
            kind: "ICAO airport code",
            input: "eddf".to_owned(),
        };
        assert_eq!(error.to_string(), "invalid ICAO airport code: \"eddf\"");
    }

    #[test]
    fn test_core_error_invalid_airport_display() {
        let error = CoreError::InvalidAirport {
            ident: "EDDF".to_owned(),
            reason: "bad name",
        };
        assert_eq!(error.to_string(), "invalid airport EDDF: bad name");
    }

    #[test]
    fn test_core_error_invalid_runway_end_display() {
        let error = CoreError::InvalidRunwayEnd {
            designator: "07L".to_owned(),
            reason: "bad",
        };
        assert_eq!(error.to_string(), "invalid runway end 07L: bad");
    }

    #[test]
    fn test_core_error_invalid_runway_display() {
        let error = CoreError::InvalidRunway {
            airport: "EDDF".to_owned(),
            designator: "07C/25C".to_owned(),
            reason: "bad",
        };
        assert_eq!(error.to_string(), "invalid runway 07C/25C at EDDF: bad");
    }

    #[test]
    fn test_core_error_duplicate_airport_display() {
        let error = CoreError::DuplicateAirport {
            ident: "EDDF".to_owned(),
        };
        assert_eq!(error.to_string(), "duplicate airport ident EDDF");
    }

    #[test]
    fn test_core_error_unknown_airport_display() {
        let error = CoreError::UnknownAirport {
            ident: "EDDH".to_owned(),
        };
        assert_eq!(error.to_string(), "runway references unknown airport EDDH");
    }

    #[test]
    fn test_core_error_unsupported_file_format_display() {
        let error = CoreError::UnsupportedFileFormat {
            file: "world file",
            found: 2,
            supported: 1,
        };
        assert_eq!(
            error.to_string(),
            "world file format 2 is not supported (expected 1)"
        );
    }

    #[test]
    fn test_core_error_invalid_aircraft_type_display() {
        let error = CoreError::InvalidAircraftType {
            code: "A320".to_owned(),
            reason: "bad",
        };
        assert_eq!(error.to_string(), "invalid aircraft type A320: bad");
    }

    #[test]
    fn test_core_error_duplicate_aircraft_type_display() {
        let error = CoreError::DuplicateAircraftType {
            code: "A320".to_owned(),
        };
        assert_eq!(error.to_string(), "duplicate aircraft type A320");
    }
}
