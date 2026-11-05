//! Planar geometry on `[lon, lat]` degree pairs, used to prepare basemap
//! layers.
//!
//! Treating degrees as planar coordinates is fine for clipping to a rectangle
//! and for choosing which points to drop when simplifying at country scale.

// Points are compared for identity (copied through unchanged), never for
// computed closeness, so exact float comparison is intended in this module.
#![allow(clippy::float_cmp)]

mod clip;
mod simplify;

pub(crate) use clip::{clip_line, clip_ring};
pub(crate) use simplify::simplify;

/// A `[lon, lat]` pair in degrees.
pub(crate) type Point = [f64; 2];

/// An axis-aligned rectangle in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub(crate) west: f64,
    pub(crate) south: f64,
    pub(crate) east: f64,
    pub(crate) north: f64,
}

impl Rect {
    /// Whether `point` is inside or on the edge, allowing for rounding.
    pub(crate) fn contains(&self, [lon, lat]: Point) -> bool {
        const SLACK: f64 = 1e-9;
        (self.west - SLACK..=self.east + SLACK).contains(&lon)
            && (self.south - SLACK..=self.north + SLACK).contains(&lat)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const GERMANY: Rect = Rect {
        west: 3.0,
        south: 45.5,
        east: 17.5,
        north: 56.5,
    };

    #[test]
    fn test_rect_contains_edges_and_rejects_outside() {
        assert!(GERMANY.contains([3.0, 45.5]));
        assert!(GERMANY.contains([10.0, 50.0]));
        assert!(!GERMANY.contains([2.9, 50.0]));
        assert!(!GERMANY.contains([10.0, 56.6]));
    }
}
