//! A chain of great-circle segments walked by distance: taxi routes on the
//! ground (plan M4).

use anyhow::Result;

use super::{LatLon, Segment};
use crate::{
    CoreError,
    units::{Degrees, Meters},
};

/// Pieces shorter than this (metres) are dropped, except a lone one.
const MIN_PIECE: f64 = 0.01;

/// Points joined by straight pieces.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Polyline {
    pieces: Vec<Segment>,
    /// Distance at which each piece starts.
    starts: Vec<f64>,
    length: f64,
}

impl Polyline {
    /// The polyline through `points`; fails without points.
    pub(crate) fn new(points: &[LatLon]) -> Result<Self> {
        let Some(&first) = points.first() else {
            return Err(CoreError::InvalidRoute {
                reason: "a polyline needs at least one point",
            }
            .into());
        };
        let mut pieces = Vec::with_capacity(points.len());
        let mut starts = Vec::with_capacity(points.len());
        let mut length = 0.0;
        for pair in points.windows(2) {
            let piece = Segment::new(pair[0], pair[1])?;
            if piece.length().value() < MIN_PIECE {
                continue;
            }
            starts.push(length);
            length += piece.length().value();
            pieces.push(piece);
        }
        if pieces.is_empty() {
            let last = points.last().copied().unwrap_or(first);
            pieces.push(Segment::new(first, last)?);
            starts.push(0.0);
            length = pieces[0].length().value();
        }
        Ok(Self {
            pieces,
            starts,
            length,
        })
    }

    /// Total length.
    pub(crate) const fn length(&self) -> Meters {
        Meters::from_raw(self.length)
    }

    fn locate(&self, distance: Meters) -> (&Segment, f64) {
        let along = distance.value().clamp(0.0, self.length);
        let index = self
            .starts
            .partition_point(|start| *start <= along)
            .saturating_sub(1)
            .min(self.pieces.len() - 1);
        (&self.pieces[index], along - self.starts[index])
    }

    /// The point `distance` along, clamped to the ends.
    pub(crate) fn point_at(&self, distance: Meters) -> LatLon {
        let (piece, along) = self.locate(distance);
        piece.point_at(Meters::from_raw(along))
    }

    /// True course `distance` along.
    pub(crate) fn bearing_at(&self, distance: Meters) -> Degrees {
        let (piece, along) = self.locate(distance);
        piece.bearing_at(Meters::from_raw(along))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_polyline_walks_its_pieces() -> Result<()> {
        let a = LatLon::new(50.0, 8.0)?;
        let b = LatLon::new(50.0, 8.01)?;
        let c = LatLon::new(50.01, 8.01)?;
        let line = Polyline::new(&[a, b, b, c])?;
        let ab = a.distance_to(b).value();
        let bc = b.distance_to(c).value();
        assert!((line.length().value() - (ab + bc)).abs() < 1e-6);
        let mid = line.point_at(Meters::new(ab / 2.0)?);
        assert!(mid.distance_to(a.interpolate(b, 0.5)?).value() < 0.01);
        assert!((line.bearing_at(Meters::new(10.0)?).value() - 90.0).abs() < 0.1);
        let north = line.bearing_at(Meters::new(ab + 10.0)?).value();
        assert!(!(0.1..=359.9).contains(&north), "{north}");
        assert!(line.point_at(Meters::new(1e9)?).distance_to(c).value() < 0.01);
        assert!(line.point_at(Meters::new(-5.0)?).distance_to(a).value() < 0.01);
        let dot = Polyline::new(&[a])?;
        assert!(dot.length().value().abs() < 1e-9);
        assert!(Polyline::new(&[]).is_err());
        assert_eq!(line.pieces.len(), 2, "the repeated point adds no piece");
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_polyline_is_continuous(
            steps in prop::collection::vec((-0.01_f64..0.01, -0.01_f64..0.01), 1..8),
            fraction in 0.0_f64..1.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let mut points = vec![LatLon::new(50.0, 8.0).map_err(fail)?];
            for (dlat, dlon) in &steps {
                let last = points[points.len() - 1];
                points.push(
                    LatLon::new(last.latitude().value() + dlat, last.longitude().value() + dlon)
                        .map_err(fail)?,
                );
            }
            let line = Polyline::new(&points).map_err(fail)?;
            let along = line.length().value() * fraction;
            let here = line.point_at(Meters::from_raw(along));
            let ahead = line.point_at(Meters::from_raw(along + 1.0));
            prop_assert!(here.distance_to(ahead).value() <= 1.0 + 1e-6);
            let walked: f64 = points.windows(2).map(|w| w[0].distance_to(w[1]).value()).sum();
            prop_assert!((walked - line.length().value()).abs() < 0.1);
        }
    }
}
