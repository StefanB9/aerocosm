//! Basemap validation errors.

use thiserror::Error;

/// Why a basemap or one of its parts is invalid.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BasemapError {
    /// The bounding box is empty or inverted.
    #[error("bounds must have south < north and west < east")]
    InvalidBounds,
    /// A polygon ring is too short or not closed.
    #[error("a ring needs at least 4 points and must be closed")]
    InvalidRing,
    /// A polygon has no rings.
    #[error("a polygon needs an outer ring")]
    EmptyPolygon,
    /// A line has fewer than two points.
    #[error("a line needs at least 2 points")]
    InvalidLine,
    /// A place or country has an empty name.
    #[error("a place needs a name")]
    UnnamedPlace,
    /// A layer has points outside the basemap bounds.
    #[error("layer {layer} has points outside the bounds")]
    OutOfBounds {
        /// The offending layer.
        layer: &'static str,
    },
    /// The file has a format version this build cannot read.
    #[error("basemap format {found} is not supported (expected {supported})")]
    UnsupportedFormat {
        /// Version found in the file.
        found: u32,
        /// Version this build reads.
        supported: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basemap_error_display() {
        let cases = [
            (
                BasemapError::InvalidBounds,
                "bounds must have south < north and west < east",
            ),
            (
                BasemapError::InvalidRing,
                "a ring needs at least 4 points and must be closed",
            ),
            (BasemapError::EmptyPolygon, "a polygon needs an outer ring"),
            (BasemapError::InvalidLine, "a line needs at least 2 points"),
            (BasemapError::UnnamedPlace, "a place needs a name"),
            (
                BasemapError::OutOfBounds { layer: "lakes" },
                "layer lakes has points outside the bounds",
            ),
            (
                BasemapError::UnsupportedFormat {
                    found: 2,
                    supported: 1,
                },
                "basemap format 2 is not supported (expected 1)",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
