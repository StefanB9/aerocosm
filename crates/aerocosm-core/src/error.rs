//! Domain errors.

use thiserror::Error;

/// Recoverable, matchable errors raised by the core library.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    /// An input failed validation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_error_invalid_input_display() {
        let error = CoreError::InvalidInput("empty name".to_owned());
        assert_eq!(error.to_string(), "invalid input: empty name");
    }
}
