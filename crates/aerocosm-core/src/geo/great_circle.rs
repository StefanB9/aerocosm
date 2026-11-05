//! Great-circle navigation on a sphere of mean Earth radius.
//!
//! A sphere is within ~0.5 % of the WGS-84 ellipsoid, which is ample for
//! traffic simulation.

use anyhow::Result;

use super::LatLon;
use crate::{
    CoreError,
    units::{Degrees, Meters},
};

/// Mean Earth radius (IUGG), in metres.
pub const EARTH_RADIUS: Meters = Meters::from_raw(6_371_008.8);

/// Below this central angle (radians, ~6 µm on the ground) two points are
/// treated as equal.
const COINCIDENT_ANGLE: f64 = 1e-12;
/// Within this central angle of π (radians, ~6 m) two points are treated as
/// antipodal. Near π the haversine term loses precision to ~1e-8 rad, so the
/// margin must exceed that.
const ANTIPODAL_MARGIN: f64 = 1e-6;

impl LatLon {
    /// Central angle to `other` in radians, `0..=π` (haversine in its `atan2`
    /// form, which is well-conditioned for both tiny and near-antipodal
    /// separations).
    fn central_angle(self, other: Self) -> f64 {
        let (lat1, lon1) = self.to_radians();
        let (lat2, lon2) = other.to_radians();
        let h = ((lat2 - lat1) / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
        let h = h.clamp(0.0, 1.0);
        2.0 * h.sqrt().atan2((1.0 - h).sqrt())
    }

    /// Great-circle distance to `other`.
    pub fn distance_to(self, other: Self) -> Meters {
        Meters::from_raw(self.central_angle(other) * EARTH_RADIUS.value())
    }

    /// Initial true course from `self` towards `other`, in `[0, 360)`. Returns
    /// 0 for coincident points.
    pub fn initial_bearing_to(self, other: Self) -> Degrees {
        let (lat1, lon1) = self.to_radians();
        let (lat2, lon2) = other.to_radians();
        let delta_lon = lon2 - lon1;
        let y = delta_lon.sin() * lat2.cos();
        let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * delta_lon.cos();
        Degrees::from_raw(y.atan2(x).to_degrees()).normalized()
    }

    /// The point reached by travelling `distance` along the great circle that
    /// starts at `self` with true course `bearing`.
    #[must_use]
    pub fn destination(self, bearing: Degrees, distance: Meters) -> Self {
        let (lat1, lon1) = self.to_radians();
        let course = bearing.to_radians();
        let angle = distance.value() / EARTH_RADIUS.value();
        let lat2 = (lat1.sin() * angle.cos() + lat1.cos() * angle.sin() * course.cos()).asin();
        let lon2 = lon1
            + (course.sin() * angle.sin() * lat1.cos())
                .atan2(angle.cos() - lat1.sin() * lat2.sin());
        Self::from_radians_unchecked(lat2, lon2)
    }

    /// The point a `fraction` (`0..=1`) of the way along the great circle to
    /// `other`.
    ///
    /// Fails for antipodal points, where the great circle is not unique.
    pub fn interpolate(self, other: Self, fraction: f64) -> Result<Self> {
        if !(0.0..=1.0).contains(&fraction) {
            return Err(CoreError::InvalidFraction { fraction }.into());
        }
        let angle = self.central_angle(other);
        if angle < COINCIDENT_ANGLE {
            return Ok(self);
        }
        if angle > std::f64::consts::PI - ANTIPODAL_MARGIN {
            return Err(CoreError::AntipodalPoints.into());
        }
        let (lat1, lon1) = self.to_radians();
        let (lat2, lon2) = other.to_radians();
        // Spherical linear interpolation of the two unit vectors.
        let weight_start = ((1.0 - fraction) * angle).sin() / angle.sin();
        let weight_end = (fraction * angle).sin() / angle.sin();
        let x = weight_start * lat1.cos() * lon1.cos() + weight_end * lat2.cos() * lon2.cos();
        let y = weight_start * lat1.cos() * lon1.sin() + weight_end * lat2.cos() * lon2.sin();
        let z = weight_start * lat1.sin() + weight_end * lat2.sin();
        Ok(Self::from_radians_unchecked(
            z.atan2(x.hypot(y)),
            y.atan2(x),
        ))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{CoreError, units::Degrees};

    fn coordinate() -> impl Strategy<Value = LatLon> {
        (-90.0_f64..=90.0, -180.0_f64..=180.0)
            .prop_filter_map("valid coordinate", |(lat, lon)| LatLon::new(lat, lon).ok())
    }

    fn fail(error: &anyhow::Error) -> TestCaseError {
        TestCaseError::fail(error.to_string())
    }

    /// Absolute tolerance in metres for round-trip checks.
    const METERS_TOLERANCE: f64 = 0.01;

    #[test]
    fn test_distance_frankfurt_munich() -> Result<()> {
        let fra = LatLon::new(50.033_333, 8.570_556)?;
        let muc = LatLon::new(48.353_783, 11.786_086)?;
        let km = fra.distance_to(muc).value() / 1000.0;
        assert!((km - 299.1).abs() < 1.5, "FRA-MUC was {km} km");
        Ok(())
    }

    #[test]
    fn test_distance_london_new_york() -> Result<()> {
        let lhr = LatLon::new(51.4700, -0.4543)?;
        let jfk = LatLon::new(40.6413, -73.7781)?;
        let km = lhr.distance_to(jfk).value() / 1000.0;
        // Ellipsoidal distance is ~5555 km; the sphere is within 0.5 %.
        assert!((km - 5555.0).abs() < 0.005 * 5555.0, "LHR-JFK was {km} km");
        Ok(())
    }

    #[test]
    fn test_initial_bearing_cardinal_directions() -> Result<()> {
        let origin = LatLon::new(0.0, 0.0)?;
        let east = origin.initial_bearing_to(LatLon::new(0.0, 10.0)?).value();
        let north = origin.initial_bearing_to(LatLon::new(10.0, 0.0)?).value();
        let west = origin.initial_bearing_to(LatLon::new(0.0, -10.0)?).value();
        assert!((east - 90.0).abs() < 1e-9);
        assert!(north.abs() < 1e-9);
        assert!((west - 270.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_destination_wraps_antimeridian() -> Result<()> {
        let start = LatLon::new(0.0, 179.0)?;
        let one_degree = Meters::new(EARTH_RADIUS.value() * 2.0_f64.to_radians())?;
        let end = start.destination(Degrees::new(90.0)?, one_degree);
        assert!((end.longitude().value() + 179.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_interpolate_rejects_bad_fraction() -> Result<()> {
        let a = LatLon::new(0.0, 0.0)?;
        let b = LatLon::new(10.0, 10.0)?;
        for fraction in [-0.1, 1.1, f64::NAN] {
            let error = a.interpolate(b, fraction).err().context("must reject")?;
            assert!(matches!(
                error.downcast_ref::<CoreError>(),
                Some(CoreError::InvalidFraction { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn test_interpolate_rejects_antipodes() -> Result<()> {
        let a = LatLon::new(10.0, 20.0)?;
        let b = LatLon::new(-10.0, -160.0)?;
        let error = a.interpolate(b, 0.5).err().context("must reject")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::AntipodalPoints)
        );
        Ok(())
    }

    #[test]
    fn test_interpolate_coincident_points() -> Result<()> {
        let a = LatLon::new(48.0, 11.0)?;
        assert!(a.distance_to(a.interpolate(a, 0.5)?).value() < METERS_TOLERANCE);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_distance_is_symmetric_and_bounded(a in coordinate(), b in coordinate()) {
            let ab = a.distance_to(b).value();
            prop_assert!((ab - b.distance_to(a).value()).abs() < METERS_TOLERANCE);
            prop_assert!(ab >= 0.0);
            prop_assert!(ab <= std::f64::consts::PI * EARTH_RADIUS.value() + METERS_TOLERANCE);
            prop_assert!(a.distance_to(a).value() < METERS_TOLERANCE);
        }

        #[test]
        fn test_distance_triangle_inequality(a in coordinate(), b in coordinate(), c in coordinate()) {
            let direct = a.distance_to(c).value();
            let via = a.distance_to(b).value() + b.distance_to(c).value();
            prop_assert!(direct <= via + METERS_TOLERANCE);
        }

        #[test]
        fn test_initial_bearing_in_range(a in coordinate(), b in coordinate()) {
            let bearing = a.initial_bearing_to(b).value();
            prop_assert!((0.0..360.0).contains(&bearing));
        }

        #[test]
        fn test_destination_travels_requested_distance(
            start in coordinate(),
            bearing in 0.0_f64..360.0,
            meters in 0.0_f64..19_000_000.0,
        ) {
            let bearing = Degrees::new(bearing).map_err(|e| fail(&e))?;
            let distance = Meters::new(meters).map_err(|e| fail(&e))?;
            let end = start.destination(bearing, distance);
            let travelled = start.distance_to(end).value();
            prop_assert!((travelled - meters).abs() < 1.0, "asked {meters}, got {travelled}");
        }

        #[test]
        fn test_interpolate_endpoints_and_proportion(
            a in coordinate(),
            b in coordinate(),
            fraction in 0.0_f64..=1.0,
        ) {
            let total = a.distance_to(b).value();
            prop_assume!(total < 0.95 * std::f64::consts::PI * EARTH_RADIUS.value());
            let start = a.interpolate(b, 0.0).map_err(|e| fail(&e))?;
            let end = a.interpolate(b, 1.0).map_err(|e| fail(&e))?;
            let mid = a.interpolate(b, fraction).map_err(|e| fail(&e))?;
            prop_assert!(a.distance_to(start).value() < 1.0);
            prop_assert!(b.distance_to(end).value() < 1.0);
            prop_assert!((a.distance_to(mid).value() - fraction * total).abs() < 1.0);
            prop_assert!((mid.distance_to(b).value() - (1.0 - fraction) * total).abs() < 1.0);
        }
    }
}
