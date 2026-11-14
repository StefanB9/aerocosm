//! Web Mercator projection around a floating origin (plan M5 D130, D131).
//!
//! Positions are projected in `f64` metres, the origin is subtracted, and only
//! the small remainder is narrowed to `f32` for the GPU. The origin follows
//! the camera ([`rebase_due`]), so drawn offsets stay small anywhere on Earth.
//! East–west offsets wrap by the world's width: everything is projected to
//! its copy nearest the origin, and drawn at its copy nearest the camera
//! ([`wrap_near`]).

use std::f64::consts::{FRAC_PI_4, PI};

use aerocosm_core::{LatLon, geo::EARTH_RADIUS};
use anyhow::Result;
use bevy::{ecs::resource::Resource, math::Vec2};

/// Radius of the Web Mercator sphere (WGS 84 semi-major axis).
const RADIUS: f64 = 6_378_137.0;
/// Latitude limit of Web Mercator, where the map becomes square.
pub(crate) const MAX_LATITUDE: f64 = 85.051_128_779_806_59;
/// East–west extent of the map, map units: the equator's length.
pub(crate) const WORLD_WIDTH: f64 = 2.0 * PI * RADIUS;
/// [`WORLD_WIDTH`] for drawing.
// SAFETY: 4e7 m in f32 is exact to a few metres, ample for wrapping.
#[allow(clippy::cast_possible_truncation)]
pub(crate) const WORLD_WIDTH_F32: f32 = WORLD_WIDTH as f32;
/// The origin moves to the camera once it is this many pixels away…
const REBASE_PIXELS: f32 = 10_000.0;
/// …or a quarter of the world, so that wrapping near the camera and near the
/// origin agree.
#[allow(clippy::cast_possible_truncation)]
const REBASE_LIMIT: f32 = (WORLD_WIDTH / 4.0) as f32;

/// Projects geographic positions to local map units (Web Mercator metres from
/// the origin).
#[derive(Debug, Clone, Copy, PartialEq, Resource)]
pub(crate) struct MapProjection {
    origin_xy: [f64; 2],
}

impl MapProjection {
    /// A projection centred on `origin`.
    pub(crate) fn new(origin: LatLon) -> Self {
        Self {
            origin_xy: mercator(origin),
        }
    }

    /// A projection centred where the equator meets the prime meridian.
    pub(crate) const fn zero() -> Self {
        Self {
            origin_xy: [0.0, 0.0],
        }
    }

    /// The point mapped to `(0, 0)`.
    #[cfg(test)]
    pub(crate) fn origin(&self) -> Result<LatLon> {
        geographic(self.origin_xy)
    }

    /// Local map position of `point`, at its copy nearest the origin.
    pub(crate) fn project(&self, point: LatLon) -> Vec2 {
        let [x, y] = mercator(point);
        narrow(wrap(x - self.origin_xy[0]), y - self.origin_xy[1])
    }

    /// Local map position of `point` without wrapping, for meshes that span
    /// the antimeridian (Natural Earth splits its shapes there).
    pub(crate) fn project_unwrapped(&self, point: LatLon) -> Vec2 {
        let [x, y] = mercator(point);
        narrow(x - self.origin_xy[0], y - self.origin_xy[1])
    }

    /// Geographic position of a local map point.
    pub(crate) fn unproject(&self, position: Vec2) -> Result<LatLon> {
        geographic([
            f64::from(position.x) + self.origin_xy[0],
            f64::from(position.y) + self.origin_xy[1],
        ])
    }

    /// Where the origin of `frame` lies in this projection, at its copy
    /// nearest the origin: the offset of a mesh built in `frame`.
    pub(crate) fn offset_to(&self, frame: &Self) -> Vec2 {
        narrow(
            wrap(frame.origin_xy[0] - self.origin_xy[0]),
            frame.origin_xy[1] - self.origin_xy[1],
        )
    }

    /// Moves the origin to the local point `to`; positions projected before
    /// move by `-to`.
    pub(crate) fn rebase(&mut self, to: Vec2) {
        self.origin_xy = [
            wrap(self.origin_xy[0] + f64::from(to.x)),
            self.origin_xy[1] + f64::from(to.y),
        ];
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

/// Whether the camera, at local point `centre` and `scale` map units per
/// pixel, is far enough from the origin to move it there.
pub(crate) fn rebase_due(centre: Vec2, scale: f32) -> bool {
    centre.length() > (REBASE_PIXELS * scale).min(REBASE_LIMIT)
}

/// `position` moved east or west by whole worlds to its copy nearest `near`.
pub(crate) fn wrap_near(position: Vec2, near: Vec2) -> Vec2 {
    #[allow(clippy::cast_possible_truncation)]
    let shift = (wrap(f64::from(position.x - near.x)) - f64::from(position.x - near.x)) as f32;
    Vec2::new(position.x + shift, position.y)
}

/// An east–west offset wrapped into half a world either way.
fn wrap(x: f64) -> f64 {
    x - WORLD_WIDTH * (x / WORLD_WIDTH).round()
}

// SAFETY: narrowing is the point of the floating origin; offsets near the
// camera are small enough for f32.
#[allow(clippy::cast_possible_truncation)]
fn narrow(x: f64, y: f64) -> Vec2 {
    Vec2::new(x as f32, y as f32)
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

/// The geographic position of Web Mercator metres, longitudes wrapped.
fn geographic([x, y]: [f64; 2]) -> Result<LatLon> {
    let lon = (wrap(x) / RADIUS).to_degrees();
    let lat = (2.0 * (y / RADIUS).exp().atan() - 2.0 * FRAC_PI_4).to_degrees();
    LatLon::new(
        lat.clamp(-MAX_LATITUDE, MAX_LATITUDE),
        lon.clamp(-180.0, 180.0),
    )
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    fn germany() -> Result<MapProjection> {
        Ok(MapProjection::new(LatLon::new(51.163_4, 10.447_7)?))
    }

    fn fail(error: impl std::fmt::Display) -> TestCaseError {
        TestCaseError::fail(error.to_string())
    }

    #[test]
    fn test_zero_projection_is_centred_on_null_island() -> Result<()> {
        let origin = MapProjection::zero().origin()?;
        assert!(origin.latitude().value().abs() < 1e-9 && origin.longitude().value().abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_projection_origin_maps_to_zero() -> Result<()> {
        let projection = germany()?;
        let origin = projection.project(projection.origin()?);
        assert!(origin.length() < 1e-3, "{origin:?}");
        Ok(())
    }

    #[test]
    fn test_projection_axes_point_east_and_north() -> Result<()> {
        let projection = germany()?;
        let centre = projection.origin()?;
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
        let projection = germany()?;
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
    fn test_points_across_the_antimeridian_are_neighbours() -> Result<()> {
        // Seen from Fiji, Samoa lies a few hundred kilometres east, not a
        // world away to the west.
        let projection = MapProjection::new(LatLon::new(-17.8, 178.0)?);
        let samoa = projection.project(LatLon::new(-13.8, -172.0)?);
        assert!(samoa.x > 0.0 && samoa.x < 1.2e6, "{samoa:?}");
        // The seam itself, from either side.
        let west = projection.project(LatLon::new(0.0, 179.999)?);
        let east = projection.project(LatLon::new(0.0, -179.999)?);
        assert!((east.x - west.x - 222.6).abs() < 1.0, "{west:?} {east:?}");
        Ok(())
    }

    #[test]
    fn test_unprojected_longitudes_wrap() -> Result<()> {
        let projection = MapProjection::new(LatLon::new(0.0, 179.0)?);
        let east = projection.unproject(Vec2::new(222_639.0, 0.0))?;
        assert!((east.longitude().value() + 179.0).abs() < 1e-3, "{east:?}");
        Ok(())
    }

    #[test]
    fn test_unwrapped_projection_keeps_a_side() -> Result<()> {
        // A mesh spanning the whole width keeps its far vertices apart.
        let projection = MapProjection::new(LatLon::new(-80.0, 0.0)?);
        let west = projection.project_unwrapped(LatLon::new(-80.0, -180.0)?);
        let east = projection.project_unwrapped(LatLon::new(-80.0, 180.0)?);
        assert!((f64::from(east.x - west.x) - WORLD_WIDTH).abs() < 10.0);
        Ok(())
    }

    #[test]
    fn test_rebase_moves_the_origin() -> Result<()> {
        let mut projection = germany()?;
        let frankfurt = LatLon::new(50.033_333, 8.570_556)?;
        let before = projection.project(frankfurt);
        projection.rebase(before);
        assert!(projection.project(frankfurt).length() < 0.01);
        let origin = projection.origin()?;
        assert!(origin.distance_to(frankfurt).value() < 0.01, "{origin:?}");
        Ok(())
    }

    #[test]
    fn test_rebase_across_the_antimeridian_wraps_the_origin() -> Result<()> {
        let mut projection = MapProjection::new(LatLon::new(0.0, 179.0)?);
        projection.rebase(Vec2::new(2.0 * 111_319.49, 0.0));
        let origin = projection.origin()?;
        assert!(
            (origin.longitude().value() + 179.0).abs() < 1e-6,
            "{origin:?}"
        );
        Ok(())
    }

    #[test]
    fn test_offset_places_an_anchor_frame() -> Result<()> {
        let projection = MapProjection::new(LatLon::new(0.0, 179.0)?);
        let anchor = MapProjection::new(LatLon::new(0.0, -179.0)?);
        let offset = projection.offset_to(&anchor);
        assert!(
            (f64::from(offset.x) - 2.0 * 111_319.49).abs() < 1.0,
            "{offset:?}"
        );
        Ok(())
    }

    #[test]
    fn test_wrap_near_picks_the_closest_copy() {
        let width = WORLD_WIDTH_F32;
        let near = Vec2::new(0.45 * width, 10.0);
        let wrapped = wrap_near(Vec2::new(-0.45 * width, 20.0), near);
        assert!((wrapped.x - 0.55 * width).abs() < 10.0, "{wrapped:?}");
        assert!((wrapped.y - 20.0).abs() < f32::EPSILON);
        assert_eq!(
            wrap_near(Vec2::new(5.0, 1.0), Vec2::ZERO),
            Vec2::new(5.0, 1.0)
        );
    }

    #[test]
    fn test_rebase_due_far_from_the_origin_in_pixels() {
        // At stand zoom (5 cm per pixel) after 500 m, at country zoom after
        // 10,000 km, at world zoom when a quarter of the world away.
        assert!(!rebase_due(Vec2::new(400.0, 0.0), 0.05));
        assert!(rebase_due(Vec2::new(600.0, 0.0), 0.05));
        assert!(!rebase_due(Vec2::new(5e6, 0.0), 1600.0));
        assert!(rebase_due(Vec2::new(1.1e7, 0.0), 30_000.0));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Anywhere on Earth, with the origin moved to the camera, a point
        /// near it round-trips within a millimetre.
        #[test]
        fn test_projection_round_trip_near_the_origin_anywhere(
            lat in -80.0_f64..80.0,
            lon in -180.0_f64..180.0,
            east in -5_000.0_f64..5_000.0,
            north in -5_000.0_f64..5_000.0,
        ) {
            let centre = LatLon::new(lat, lon).map_err(fail)?;
            let projection = MapProjection::new(centre);
            let point = centre
                .destination(
                    aerocosm_core::units::Degrees::new(east.atan2(north).to_degrees().rem_euclid(360.0)).map_err(fail)?,
                    aerocosm_core::units::Meters::new(east.hypot(north)).map_err(fail)?,
                );
            let back = projection.unproject(projection.project(point)).map_err(fail)?;
            prop_assert!(back.distance_to(point).value() < 1e-3, "{} m", back.distance_to(point).value());
        }

        /// After any sequence of rebases the projection agrees with a fresh
        /// one at the same origin (north–south within the map, where the
        /// camera keeps it).
        #[test]
        fn test_rebases_compose(
            steps in proptest::collection::vec((-1.5e7_f32..1.5e7, -2e6_f32..2e6), 1..8),
            lat in -60.0_f64..60.0,
            lon in -180.0_f64..180.0,
        ) {
            let mut projection = MapProjection::new(LatLon::new(0.0, 0.0).map_err(fail)?);
            for (x, y) in steps {
                projection.rebase(Vec2::new(x, y));
            }
            let fresh = MapProjection::new(projection.origin().map_err(fail)?);
            let point = LatLon::new(lat, lon).map_err(fail)?;
            let (a, b) = (projection.project(point), fresh.project(point));
            prop_assert!(a.distance(b) < 2.0, "{a:?} {b:?}");
        }

        /// Projected offsets never exceed half the world east or west.
        #[test]
        fn test_projected_offsets_stay_within_half_the_world(
            a in -180.0_f64..180.0,
            b in -180.0_f64..180.0,
            lat in -60.0_f64..60.0,
        ) {
            let projection = MapProjection::new(LatLon::new(lat, a).map_err(fail)?);
            let x = f64::from(projection.project(LatLon::new(lat, b).map_err(fail)?).x);
            prop_assert!(x.abs() <= WORLD_WIDTH / 2.0 + 1.0, "{x}");
        }

        #[test]
        fn test_wrap_near_stays_within_half_the_world(x in -6e7_f32..6e7, near in -1e7_f32..1e7) {
            let wrapped = wrap_near(Vec2::new(x, 0.0), Vec2::new(near, 0.0));
            prop_assert!((wrapped.x - near).abs() <= WORLD_WIDTH_F32 / 2.0 + 8.0);
            // Moved by whole worlds only.
            let worlds = f64::from(wrapped.x - x) / WORLD_WIDTH;
            prop_assert!((worlds - worlds.round()).abs() < 1e-5);
        }

        #[test]
        fn test_projection_preserves_order(a in -80.0_f64..80.0, b in -80.0_f64..80.0, lon in -180.0_f64..180.0) {
            prop_assume!((a - b).abs() > 1e-4);
            let projection = germany().map_err(fail)?;
            let pa = projection.project(LatLon::new(a, lon).map_err(fail)?);
            let pb = projection.project(LatLon::new(b, lon).map_err(fail)?);
            prop_assert_eq!(a < b, pa.y < pb.y);
        }
    }
}
