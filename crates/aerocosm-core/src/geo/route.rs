//! A route through waypoints with its corners rounded into arcs (plan M3
//! D59), walked by distance like a single segment.

use anyhow::Result;

use super::{LatLon, Segment};
use crate::{
    CoreError,
    units::{Degrees, Meters},
};

/// Straight pieces shorter than this (metres) are dropped: a corner's arc may
/// use up a whole leg.
const MIN_LINE: f64 = 0.01;

/// Corners turning less than this are left sharp.
const MIN_TURN_DEGREES: f64 = 1.0;

/// One piece of a route.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Leg {
    /// A great-circle segment.
    Line(Segment),
    /// A constant-radius turn about `center`, starting at radial bearing
    /// `start` (degrees from the centre to the start point) and sweeping
    /// `sweep` degrees, positive clockwise.
    Arc {
        center: LatLon,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

impl Leg {
    fn length(&self) -> f64 {
        match self {
            Self::Line(segment) => segment.length().value(),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs().to_radians(),
        }
    }

    fn point_at(&self, along: f64) -> LatLon {
        match self {
            Self::Line(segment) => segment.point_at(Meters::from_raw(along)),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => center.destination(
                Degrees::from_raw(start + sweep.signum() * (along / radius).to_degrees())
                    .normalized(),
                Meters::from_raw(*radius),
            ),
        }
    }

    fn bearing_at(&self, along: f64) -> Degrees {
        match self {
            Self::Line(segment) => segment.bearing_at(Meters::from_raw(along)),
            Self::Arc {
                radius,
                start,
                sweep,
                ..
            } => {
                let radial = start + sweep.signum() * (along / radius).to_degrees();
                Degrees::from_raw(radial + sweep.signum() * 90.0).normalized()
            }
        }
    }
}

/// Appends the great circle from `from` to `to` unless it is negligibly short.
fn push_line(legs: &mut Vec<Leg>, from: LatLon, to: LatLon) -> Result<()> {
    if from.distance_to(to).value() >= MIN_LINE {
        legs.push(Leg::Line(Segment::new(from, to)?));
    }
    Ok(())
}

/// Signed turn from course `a` to course `b`, degrees in `(-180, 180]`,
/// positive clockwise.
fn turn(a: Degrees, b: Degrees) -> f64 {
    let turn = (b.value() - a.value()).rem_euclid(360.0);
    if turn > 180.0 { turn - 360.0 } else { turn }
}

/// Waypoints joined by great circles, each inner corner rounded by an arc of
/// the given radius, or tighter where the adjacent legs are too short.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Route {
    legs: Vec<Leg>,
    /// Distance at which each leg starts.
    starts: Vec<f64>,
    length: f64,
}

impl Route {
    /// Rounds the corners of `points` with arcs of `radius`.
    pub(crate) fn new(points: &[LatLon], radius: Meters) -> Result<Self> {
        if points.len() < 2 {
            return Err(CoreError::InvalidRoute {
                reason: "a route needs at least two points",
            }
            .into());
        }
        // The point each straight leg starts from; corners move it forward.
        let mut legs = Vec::with_capacity(points.len() * 2);
        let mut from = points[0];
        for (index, corner) in points.iter().enumerate().skip(1) {
            let Some(next) = points.get(index + 1) else {
                push_line(&mut legs, from, *corner)?;
                break;
            };
            let incoming =
                Degrees::from_raw(corner.initial_bearing_to(from).value() + 180.0).normalized();
            let outgoing = corner.initial_bearing_to(*next);
            let angle = turn(incoming, outgoing);
            if angle.abs() < MIN_TURN_DEGREES {
                push_line(&mut legs, from, *corner)?;
                from = *corner;
                continue;
            }
            // Tangent length r·tan(θ/2), limited to half of each adjacent leg.
            let half = (angle.abs() / 2.0).to_radians().tan();
            let room = from
                .distance_to(*corner)
                .value()
                .min(corner.distance_to(*next).value() / 2.0);
            let tangent = (radius.value() * half).min(room);
            let fitted = tangent / half;
            let entry = corner.destination(
                Degrees::from_raw(incoming.value() + 180.0).normalized(),
                Meters::from_raw(tangent),
            );
            let side = angle.signum();
            let center = entry.destination(
                Degrees::from_raw(incoming.value() + side * 90.0).normalized(),
                Meters::from_raw(fitted),
            );
            let arc = Leg::Arc {
                center,
                radius: fitted,
                start: center.initial_bearing_to(entry).value(),
                sweep: angle,
            };
            push_line(&mut legs, from, entry)?;
            // The next leg starts where the arc really ends, which keeps the
            // route continuous on the sphere.
            from = arc.point_at(arc.length());
            legs.push(arc);
        }
        if legs.is_empty() {
            legs.push(Leg::Line(Segment::new(points[0], points[0])?));
        }
        let mut starts = Vec::with_capacity(legs.len());
        let mut length = 0.0;
        for leg in &legs {
            starts.push(length);
            length += leg.length();
        }
        Ok(Self {
            legs,
            starts,
            length,
        })
    }

    /// Total length.
    pub(crate) fn length(&self) -> Meters {
        Meters::from_raw(self.length)
    }

    fn locate(&self, distance: Meters) -> (&Leg, f64) {
        let along = distance.value().clamp(0.0, self.length);
        let index = self
            .starts
            .partition_point(|start| *start <= along)
            .saturating_sub(1);
        let leg = &self.legs[index.min(self.legs.len() - 1)];
        (leg, along - self.starts[index.min(self.starts.len() - 1)])
    }

    /// The point `distance` along the route, clamped to its ends.
    pub(crate) fn point_at(&self, distance: Meters) -> LatLon {
        let (leg, along) = self.locate(distance);
        leg.point_at(along)
    }

    /// True course `distance` along the route.
    pub(crate) fn bearing_at(&self, distance: Meters) -> Degrees {
        let (leg, along) = self.locate(distance);
        leg.bearing_at(along)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use proptest::prelude::*;

    use super::*;

    fn turn(a: Degrees, b: Degrees) -> f64 {
        (b.value() - a.value() + 540.0).rem_euclid(360.0) - 180.0
    }

    #[test]
    fn test_straight_route_is_one_segment() -> Result<()> {
        let a = LatLon::new(50.0, 8.0)?;
        let b = LatLon::new(50.0, 9.0)?;
        let route = Route::new(&[a, b], Meters::new(2_500.0)?)?;
        assert!((route.length().value() - a.distance_to(b).value()).abs() < 1e-6);
        assert!(route.point_at(Meters::new(0.0)?).distance_to(a).value() < 1e-3);
        assert!(route.point_at(route.length()).distance_to(b).value() < 1e-3);
        Ok(())
    }

    #[test]
    fn test_corner_becomes_a_tangent_arc() -> Result<()> {
        let a = LatLon::new(50.0, 8.0)?;
        let corner = a.destination(Degrees::new(90.0)?, Meters::new(30_000.0)?);
        let c = corner.destination(Degrees::new(180.0)?, Meters::new(30_000.0)?);
        let radius = Meters::new(2_500.0)?;
        let route = Route::new(&[a, corner, c], radius)?;
        // Cutting the corner saves 2r - πr/2 of the 60 km.
        let saved = 2.0 * 2_500.0 - std::f64::consts::FRAC_PI_2 * 2_500.0;
        assert!(
            // The great circle east does not meet the southbound one at exactly
            // 90°.
            (route.length().value() - (60_000.0 - saved)).abs() < 30.0,
            "{} vs {}",
            route.length().value(),
            60_000.0 - saved
        );
        // The course turns smoothly from east to south.
        let mid = route.length() * 0.5;
        let course = route.bearing_at(mid).value();
        assert!((course - 135.0).abs() < 2.0, "{course}");
        // The corner itself is not flown over.
        assert!(route.point_at(mid).distance_to(corner).value() > 500.0);
        Ok(())
    }

    #[test]
    fn test_route_rejects_too_few_points() -> Result<()> {
        if Route::new(&[LatLon::new(50.0, 8.0)?], Meters::new(1.0)?).is_ok() {
            bail!("a route needs two points");
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_routes_are_continuous_and_smooth(
            lat in 45.0_f64..55.0, lon in 2.0_f64..16.0,
            first in 0.0_f64..360.0,
            legs in prop::collection::vec((-150.0_f64..150.0, 3_000.0_f64..80_000.0), 1..5),
            radius in 500.0_f64..4_000.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let mut points = vec![LatLon::new(lat, lon).map_err(fail)?];
            let mut heading = first;
            for (turn_by, distance) in &legs {
                heading = (heading + turn_by).rem_euclid(360.0);
                let last = points[points.len() - 1];
                points.push(last.destination(
                    Degrees::new(heading).map_err(fail)?,
                    Meters::new(*distance).map_err(fail)?,
                ));
            }
            let route = Route::new(&points, Meters::new(radius).map_err(fail)?).map_err(fail)?;
            let first = points[0];
            let last = points[points.len() - 1];
            prop_assert!(route.point_at(Meters::default()).distance_to(first).value() < 1.0);
            prop_assert!(route.point_at(route.length()).distance_to(last).value() < 1.0);
            let step = 50.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let steps = (route.length().value() / step) as usize;
            let mut previous = route.point_at(Meters::default());
            let mut course = route.bearing_at(Meters::default());
            for i in 1..=steps {
                #[allow(clippy::cast_precision_loss)]
                let along = Meters::new(step * i as f64).map_err(fail)?;
                let here = route.point_at(along);
                // Moving `step` along the route moves at most `step` on the ground.
                prop_assert!(previous.distance_to(here).value() <= step + 0.5);
                let next = route.bearing_at(along);
                // No course jumps: corners of up to 150° on legs of 3 km or more
                // leave arcs of at least ~400 m radius, about 7° per step.
                prop_assert!(turn(course, next).abs() < 20.0, "{:?}", (course, next));
                previous = here;
                course = next;
            }
        }
    }
}
