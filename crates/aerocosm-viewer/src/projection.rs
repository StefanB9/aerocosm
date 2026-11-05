//! Web Mercator projection around a fixed local origin (plan D25).
//!
//! Positions are projected in `f64` metres, the origin is subtracted, and only
//! the small remainder is narrowed to `f32` for the GPU. With the origin at
//! Germany's centre every point of the map is within ~1e6 m, where `f32`
//! resolves ~0.1 m.

use std::f64::consts::FRAC_PI_4;

use aerocosm_core::{LatLon, geo::EARTH_RADIUS};
use anyhow::Result;
use bevy::{ecs::resource::Resource, math::Vec2};

/// Radius of the Web Mercator sphere (WGS 84 semi-major axis).
const RADIUS: f64 = 6_378_137.0;
/// Latitude limit of Web Mercator, where the map becomes square.
const MAX_LATITUDE: f64 = 85.051_128_779_806_59;

/// Projects geographic positions to local map units (Web Mercator metres from
/// the origin).
#[derive(Debug, Clone, Copy, PartialEq, Resource)]
pub(crate) struct MapProjection {
    origin: LatLon,
    origin_xy: [f64; 2],
}

impl MapProjection {
    /// A projection centred on `origin`.
    pub(crate) fn new(origin: LatLon) -> Self {
        Self {
            origin,
            origin_xy: mercator(origin),
        }
    }

    /// Centred on Germany's geographic centre.
    pub(crate) fn germany() -> Result<Self> {
        Ok(Self::new(LatLon::new(51.163_4, 10.447_7)?))
    }

    /// The point mapped to `(0, 0)`.
    #[cfg(test)]
    pub(crate) const fn origin(&self) -> LatLon {
        self.origin
    }

    /// Local map position of `point`.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn project(&self, point: LatLon) -> Vec2 {
        let [x, y] = mercator(point);
        // SAFETY: narrowing is the point of the local origin; offsets are small
        // enough for f32 to resolve ~0.1 m.
        Vec2::new(
            (x - self.origin_xy[0]) as f32,
            (y - self.origin_xy[1]) as f32,
        )
    }

    /// Geographic position of a local map point.
    pub(crate) fn unproject(&self, position: Vec2) -> Result<LatLon> {
        let x = f64::from(position.x) + self.origin_xy[0];
        let y = f64::from(position.y) + self.origin_xy[1];
        let lon = (x / RADIUS).to_degrees();
        let lat = (2.0 * (y / RADIUS).exp().atan() - 2.0 * FRAC_PI_4).to_degrees();
        LatLon::new(
            lat.clamp(-MAX_LATITUDE, MAX_LATITUDE),
            lon.clamp(-180.0, 180.0),
        )
    }

    /// Web Mercator stretch at `point`: `1 / cos(lat)`.
    pub(crate) fn scale_factor(point: LatLon) -> f64 {
        1.0 / point.latitude().value().to_radians().cos()
    }

    /// Map units per ground metre at `point`, consistent with projected
    /// distances: the Mercator stretch times the ratio of the Web Mercator
    /// radius to the mean Earth radius that core geodesy computes positions on.
    pub(crate) fn units_per_meter(point: LatLon) -> f64 {
        Self::scale_factor(point) * RADIUS / EARTH_RADIUS.value()
    }
}

/// Web Mercator metres of `point`.
fn mercator(point: LatLon) -> [f64; 2] {
    let lat = point
        .latitude()
        .value()
        .clamp(-MAX_LATITUDE, MAX_LATITUDE)
        .to_radians();
    let x = RADIUS * point.longitude().value().to_radians();
    let y = RADIUS * (FRAC_PI_4 + lat / 2.0).tan().ln();
    [x, y]
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_projection_origin_maps_to_zero() -> Result<()> {
        let projection = MapProjection::germany()?;
        let origin = projection.project(projection.origin());
        assert!(origin.length() < 1e-6, "{origin:?}");
        Ok(())
    }

    #[test]
    fn test_projection_axes_point_east_and_north() -> Result<()> {
        let projection = MapProjection::germany()?;
        let centre = projection.origin();
        let east = projection.project(LatLon::new(
            centre.latitude().value(),
            centre.longitude().value() + 1.0,
        )?);
        let north = projection.project(LatLon::new(
            centre.latitude().value() + 1.0,
            centre.longitude().value(),
        )?);
        assert!(east.x > 0.0 && east.y.abs() < 1e-3);
        assert!(north.y > 0.0 && north.x.abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn test_projection_one_degree_of_longitude_at_equator() -> Result<()> {
        let projection = MapProjection::new(LatLon::new(0.0, 0.0)?);
        let east = projection.project(LatLon::new(0.0, 1.0)?);
        // 2π·6378137 / 360 m per degree on the Web Mercator sphere.
        assert!((f64::from(east.x) - 111_319.49).abs() < 0.1, "{}", east.x);
        Ok(())
    }

    #[test]
    fn test_scale_factor_grows_with_latitude() -> Result<()> {
        assert!((MapProjection::scale_factor(LatLon::new(0.0, 0.0)?) - 1.0).abs() < 1e-12);
        let at_60 = MapProjection::scale_factor(LatLon::new(60.0, 0.0)?);
        assert!((at_60 - 2.0).abs() < 1e-9, "{at_60}");
        Ok(())
    }

    #[test]
    fn test_units_per_meter_matches_projected_distances() -> Result<()> {
        let projection = MapProjection::germany()?;
        let start = LatLon::new(50.0, 8.5)?;
        let end = start.destination(
            aerocosm_core::units::Degrees::new(90.0)?,
            aerocosm_core::units::Meters::new(1000.0)?,
        );
        let projected = f64::from(projection.project(end).distance(projection.project(start)));
        assert!(
            (projected / MapProjection::units_per_meter(start) - 1000.0).abs() < 0.5,
            "{projected}"
        );
        Ok(())
    }

    #[test]
    fn test_unproject_rejects_nothing_inside_germany() -> Result<()> {
        let projection = MapProjection::germany()?;
        let frankfurt = LatLon::new(50.033_333, 8.570_556)?;
        let back = projection.unproject(projection.project(frankfurt))?;
        assert!(back.distance_to(frankfurt).value() < 0.5);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_projection_round_trip_within_decimetres(lat in 45.0_f64..57.0, lon in 2.0_f64..19.0) {
            let projection = MapProjection::germany().map_err(|e| TestCaseError::fail(e.to_string()))?;
            let point = LatLon::new(lat, lon).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let back = projection.unproject(projection.project(point)).map_err(|e| TestCaseError::fail(e.to_string()))?;
            // f32 at ~1e6 m resolves ~0.06-0.125 m; allow a little more for the round trip.
            prop_assert!(back.distance_to(point).value() < 0.25, "{} m", back.distance_to(point).value());
        }

        #[test]
        fn test_projection_preserves_order(a in 45.0_f64..57.0, b in 45.0_f64..57.0, lon in 2.0_f64..19.0) {
            prop_assume!((a - b).abs() > 1e-4);
            let projection = MapProjection::germany().map_err(|e| TestCaseError::fail(e.to_string()))?;
            let pa = projection.project(LatLon::new(a, lon).map_err(|e| TestCaseError::fail(e.to_string()))?);
            let pb = projection.project(LatLon::new(b, lon).map_err(|e| TestCaseError::fail(e.to_string()))?);
            prop_assert_eq!(a < b, pa.y < pb.y);
        }
    }
}
