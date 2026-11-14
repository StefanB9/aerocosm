//! The selected aircraft's taxi route (plan M4 D86, D87): the stretch it is
//! cleared for, the rest of the way, and its clearance limit.

use aerocosm_core::units::Meters;
use bevy::{color::Alpha, ecs::system::Res, gizmos::gizmos::Gizmos, math::Vec2};

use crate::{
    camera::CameraRig,
    driver::SimDriver,
    projection::{MapProjection, wrap_near},
    selection::Selection,
    theme,
};

/// The clearance limit's marker radius, in pixels.
const LIMIT_RADIUS: f32 = 5.0;

/// A taxi route split where the aircraft is and where its clearance ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RouteSplit {
    /// From the aircraft to its clearance limit.
    pub(crate) cleared: Vec<Vec2>,
    /// From the clearance limit to the end of the route.
    pub(crate) ahead: Vec<Vec2>,
    /// The clearance limit, short of the route's end.
    pub(crate) limit: Option<Vec2>,
}

/// The part of the polyline `points`, at cumulative distances `distances`,
/// between `from` and `to`.
fn piece(points: &[Vec2], distances: &[f32], from: f32, to: f32) -> Vec<Vec2> {
    let at = |d: f32| {
        let i = distances
            .partition_point(|&x| x < d)
            .clamp(1, points.len() - 1);
        let (a, b) = (distances[i - 1], distances[i]);
        let t = if b > a {
            ((d - a) / (b - a)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        points[i - 1].lerp(points[i], t)
    };
    let mut piece = Vec::with_capacity(points.len());
    if to <= from || points.len() < 2 {
        return piece;
    }
    piece.push(at(from));
    piece.extend(
        points
            .iter()
            .zip(distances)
            .filter(|&(_, &d)| d > from && d < to)
            .map(|(p, _)| *p),
    );
    piece.push(at(to));
    piece
}

/// Splits the route `points` (cumulative `distances` along it) at `along`,
/// where the aircraft is, and `cleared`, its clearance limit, up to `end`.
pub(crate) fn split_route(
    points: &[Vec2],
    distances: &[f32],
    along: f32,
    cleared: f32,
    end: f32,
) -> RouteSplit {
    let end = end.min(distances.last().copied().unwrap_or(0.0));
    let cleared = cleared.clamp(along, end.max(along));
    RouteSplit {
        cleared: piece(points, distances, along, cleared),
        ahead: piece(points, distances, cleared, end),
        limit: (cleared + 1.0 < end)
            .then(|| piece(points, distances, cleared, end).first().copied())
            .flatten(),
    }
}

/// Draws the selected aircraft's taxi route while it pushes back or taxies.
pub(super) fn draw_taxi_route(
    rig: Option<Res<CameraRig>>,
    driver: Res<SimDriver>,
    selection: Res<Selection>,
    projection: Res<MapProjection>,
    mut gizmos: Gizmos,
) {
    let (Some(rig), Some(tail)) = (rig, selection.selected_flight()) else {
        return;
    };
    let Some(route) = driver.simulation().taxi_route(tail) else {
        return;
    };
    let points: Vec<Vec2> = route
        .points()
        .iter()
        .map(|p| wrap_near(projection.project(*p), rig.state.center()))
        .collect();
    let mut distances = Vec::with_capacity(points.len());
    let mut walked = 0.0_f64;
    for (i, point) in route.points().iter().enumerate() {
        if i > 0 {
            walked += route.points()[i - 1].distance_to(*point).value();
        }
        // SAFETY: airport distances, a few kilometres.
        #[allow(clippy::cast_possible_truncation)]
        distances.push(walked as f32);
    }
    let metres = |m: Meters| {
        #[allow(clippy::cast_possible_truncation)]
        let m = m.value() as f32;
        m
    };
    let split = split_route(
        &points,
        &distances,
        metres(route.along()),
        metres(route.cleared()),
        metres(route.end()),
    );
    gizmos.linestrip_2d(split.ahead, theme::TEXT.with_alpha(0.35));
    gizmos.linestrip_2d(split.cleared, theme::ACCENT);
    if let Some(limit) = split.limit {
        gizmos.circle_2d(limit, LIMIT_RADIUS * rig.state.scale(), theme::ACCENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An L: 100 m east, then 100 m north.
    fn ell() -> (Vec<Vec2>, Vec<f32>) {
        (
            vec![Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)],
            vec![0.0, 100.0, 200.0],
        )
    }

    #[test]
    fn test_split_route_at_the_aircraft_and_its_limit() {
        let (points, distances) = ell();
        let split = split_route(&points, &distances, 50.0, 150.0, 200.0);
        assert_eq!(
            split.cleared,
            [
                Vec2::new(50.0, 0.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(100.0, 50.0)
            ]
        );
        assert_eq!(
            split.ahead,
            [Vec2::new(100.0, 50.0), Vec2::new(100.0, 100.0)]
        );
        assert_eq!(split.limit, Some(Vec2::new(100.0, 50.0)));
    }

    #[test]
    fn test_split_route_cleared_to_the_end_has_no_limit() {
        let (points, distances) = ell();
        let split = split_route(&points, &distances, 10.0, 500.0, 200.0);
        assert_eq!(split.cleared.len(), 3);
        assert_eq!(split.cleared.last(), Some(&Vec2::new(100.0, 100.0)));
        assert_eq!(split.ahead, []);
        assert_eq!(split.limit, None);
    }

    #[test]
    fn test_split_route_holding_at_its_limit_shows_only_the_way_ahead() {
        let (points, distances) = ell();
        let split = split_route(&points, &distances, 120.0, 0.0, 200.0);
        assert_eq!(split.cleared, []);
        assert_eq!(split.ahead.first(), Some(&Vec2::new(100.0, 20.0)));
        assert_eq!(split.limit, Some(Vec2::new(100.0, 20.0)));
        assert_eq!(split_route(&[], &[], 0.0, 0.0, 0.0), RouteSplit::default());
    }
}
