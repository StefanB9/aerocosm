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
}
