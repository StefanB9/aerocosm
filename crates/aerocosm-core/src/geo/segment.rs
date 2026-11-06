//! A great-circle segment parameterised by distance flown (D48): positions
//! cost two trigonometric calls and are evaluated only when needed.

use anyhow::Result;

use super::{EARTH_RADIUS, LatLon};
use crate::{
    CoreError,
    units::{Degrees, Meters},
};

/// Within this central angle of π the great circle is not unique.
const ANTIPODAL_MARGIN: f64 = 1e-6;
/// Below this central angle (radians) the ends coincide.
const COINCIDENT_ANGLE: f64 = 1e-12;

type Vector = [f64; 3];

fn unit(point: LatLon) -> Vector {
    let (lat, lon) = point.to_radians();
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn combine(a: Vector, wa: f64, b: Vector, wb: f64) -> Vector {
    [
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
    ]
}

/// The great circle from one point to another, walked by distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    start: Vector,
    /// Unit vector perpendicular to `start` in the plane of travel.
    tangent: Vector,
    /// Central angle from start to end, radians.
    angle: f64,
}

impl Segment {
    /// The segment from `from` to `to`; fails for antipodal points.
    pub(crate) fn new(from: LatLon, to: LatLon) -> Result<Self> {
        let start = unit(from);
        let end = unit(to);
        let angle = from.distance_to(to).value() / EARTH_RADIUS.value();
        if angle > std::f64::consts::PI - ANTIPODAL_MARGIN {
            return Err(CoreError::AntipodalPoints.into());
        }
        let tangent = if angle < COINCIDENT_ANGLE {
            [0.0; 3]
        } else {
            // `end` minus its component along `start`, normalised.
            let along = combine(end, 1.0, start, -angle.cos());
            let norm = dot(along, along).sqrt();
            [along[0] / norm, along[1] / norm, along[2] / norm]
        };
        Ok(Self {
            start,
            tangent,
            angle,
        })
    }

    /// Length along the great circle.
    pub(crate) fn length(&self) -> Meters {
        Meters::from_raw(self.angle * EARTH_RADIUS.value())
    }

    fn theta(&self, distance: Meters) -> f64 {
        (distance.value() / EARTH_RADIUS.value()).clamp(0.0, self.angle)
    }

    /// The point `distance` along the segment, clamped to its ends.
    pub(crate) fn point_at(&self, distance: Meters) -> LatLon {
        let theta = self.theta(distance);
        let p = combine(self.start, theta.cos(), self.tangent, theta.sin());
        LatLon::from_radians_unchecked(p[2].clamp(-1.0, 1.0).asin(), p[1].atan2(p[0]))
    }

    /// True course at `distance` along the segment, `[0, 360)`.
    pub(crate) fn bearing_at(&self, distance: Meters) -> Degrees {
        let theta = self.theta(distance);
        let p = combine(self.start, theta.cos(), self.tangent, theta.sin());
        let direction = combine(self.start, -theta.sin(), self.tangent, theta.cos());
        let lon = p[1].atan2(p[0]);
        let lat = p[2].clamp(-1.0, 1.0).asin();
        let east = [-lon.sin(), lon.cos(), 0.0];
        let north = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
        Degrees::from_raw(
            dot(direction, east)
                .atan2(dot(direction, north))
                .to_degrees(),
        )
        .normalized()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use proptest::prelude::*;

    use super::*;

    fn close(a: LatLon, b: LatLon, meters: f64) -> bool {
        a.distance_to(b).value() < meters
    }

    #[test]
    fn test_segment_frankfurt_to_munich() -> Result<()> {
        let frankfurt = LatLon::new(50.0333, 8.5706)?;
        let munich = LatLon::new(48.3538, 11.7861)?;
        let segment = Segment::new(frankfurt, munich)?;
        let length = frankfurt.distance_to(munich);
        assert!((segment.length().value() - length.value()).abs() < 1e-6);
        assert!(close(segment.point_at(Meters::new(0.0)?), frankfurt, 1e-3));
        assert!(close(segment.point_at(length), munich, 1e-3));
        // Beyond the ends the segment clamps.
        assert!(close(segment.point_at(length * 2.0), munich, 1e-3));
        let bearing = segment.bearing_at(Meters::new(0.0)?).value();
        assert!((bearing - frankfurt.initial_bearing_to(munich).value()).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn test_segment_of_coincident_points_has_zero_length() -> Result<()> {
        let point = LatLon::new(52.0, 13.0)?;
        let segment = Segment::new(point, point)?;
        assert!(segment.length().value().abs() < 1e-9);
        assert!(close(segment.point_at(Meters::new(10.0)?), point, 1e-3));
        Ok(())
    }

    #[test]
    fn test_segment_rejects_antipodal_points() -> Result<()> {
        if Segment::new(LatLon::new(0.0, 0.0)?, LatLon::new(0.0, -180.0)?).is_ok() {
            bail!("antipodal points have no unique great circle");
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_segment_matches_great_circle_maths(
            lat1 in -70.0_f64..70.0, lon1 in -180.0_f64..180.0,
            lat2 in -70.0_f64..70.0, lon2 in -180.0_f64..180.0,
            fraction in 0.0_f64..=1.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let a = LatLon::new(lat1, lon1).map_err(fail)?;
            let b = LatLon::new(lat2, lon2).map_err(fail)?;
            prop_assume!(a.distance_to(b).value() > 1_000.0);
            prop_assume!(a.distance_to(b).value() < 19_000_000.0);
            let segment = Segment::new(a, b).map_err(fail)?;
            let along = segment.length() * fraction;
            let point = segment.point_at(along);
            prop_assert!((a.distance_to(point).value() - along.value()).abs() < 1.0);
            let expected = a.interpolate(b, fraction).map_err(fail)?;
            prop_assert!(close(point, expected, 1.0));
            // The course at a point is the initial course from it to the end.
            prop_assume!(fraction < 0.99);
            let course = segment.bearing_at(along).value();
            let towards = point.initial_bearing_to(b).value();
            let difference = Degrees::new(course - towards + 180.0).map_err(fail)?.normalized();
            prop_assert!((difference.value() - 180.0).abs() < 1e-3);
        }
    }
}
