//! Simple departure and arrival procedures (plan M3 D59): climb out on the
//! runway heading before turning; join the final through an intercept point,
//! by downwind and base when arriving from ahead.

use super::motion::FINAL_LENGTH as FINAL;
use crate::{
    LatLon,
    units::{Degrees, Meters},
};

/// Departures climb out at least this far on the runway heading (4 NM)…
const MIN_CLIMB_OUT: f64 = 7_408.0;
/// …and until 2,000 ft above the field.
const CLIMB_OUT_HEIGHT: f64 = 609.6;
/// The intercept point lies this far beyond the final approach fix (3 NM).
const INTERCEPT: f64 = 5_556.0;
/// Downwind is flown this far to the side of the final course (3 NM)…
const DOWNWIND_OFFSET: f64 = 5_556.0;
/// …joined at most this far beyond the threshold (about 11 NM).
const DOWNWIND_MAX_AHEAD: f64 = 20_000.0;

/// Signed turn from course `a` to course `b`, degrees in `(-180, 180]`.
fn turn(a: Degrees, b: Degrees) -> f64 {
    let turn = (b.value() - a.value()).rem_euclid(360.0);
    if turn > 180.0 { turn - 360.0 } else { turn }
}

fn course(value: f64) -> Degrees {
    Degrees::from_raw(value).normalized()
}

/// Distance flown on the runway heading after lift-off: until 2,000 ft above
/// the field at `climb_rate` (m/s) and `speed` (m/s), at least 4 NM.
pub(crate) fn climb_out_distance(climb_rate: f64, speed: f64) -> Meters {
    Meters::from_raw((CLIMB_OUT_HEIGHT / climb_rate.max(0.1) * speed).max(MIN_CLIMB_OUT))
}

/// Where a departure turns onto course: `distance` ahead of lift-off on the
/// runway `heading`.
pub(crate) fn departure_turn_point(liftoff: LatLon, heading: Degrees, distance: Meters) -> LatLon {
    liftoff.destination(heading, distance)
}

/// Waypoints from the en-route phase to the final approach fix of a runway
/// with landing `threshold` and final `course`, for an aircraft coming from
/// `from`: the intercept point and the fix, preceded by downwind and base
/// when a straight join would turn more than 90°.
pub(crate) fn arrival_points(
    from: LatLon,
    threshold: LatLon,
    final_course: Degrees,
) -> Vec<LatLon> {
    let outbound = course(final_course.value() + 180.0);
    let fix = threshold.destination(outbound, Meters::from_raw(FINAL));
    let intercept = threshold.destination(outbound, Meters::from_raw(FINAL + INTERCEPT));
    let incoming = course(intercept.initial_bearing_to(from).value() + 180.0);
    if turn(incoming, final_course).abs() <= 90.0 {
        return vec![intercept, fix];
    }
    // Downwind on the side the aircraft is on, from abeam the far end of the
    // final back to a base turn abeam the intercept point.
    let relative = turn(final_course, threshold.initial_bearing_to(from));
    let side = if relative >= 0.0 { 1.0 } else { -1.0 };
    let abeam = course(final_course.value() + side * 90.0);
    // Along-track position relative to the threshold, positive ahead of it.
    let along = threshold.distance_to(from).value() * relative.to_radians().cos();
    let base = intercept.destination(abeam, Meters::from_raw(DOWNWIND_OFFSET));
    let base_along = -(FINAL + INTERCEPT);
    if along <= base_along + DOWNWIND_OFFSET {
        // Already level with or behind the base turn: join base directly.
        return vec![base, intercept, fix];
    }
    // Join downwind abeam the aircraft's own along-track position, so the
    // turn onto downwind is a right angle; aircraft far ahead join it at its
    // far end, turning less.
    let downwind = threshold
        .destination(
            final_course,
            Meters::from_raw(along.min(DOWNWIND_MAX_AHEAD)),
        )
        .destination(abeam, Meters::from_raw(DOWNWIND_OFFSET));
    vec![downwind, base, intercept, fix]
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    /// Signed turn between two courses, degrees in (-180, 180].
    fn turn(a: Degrees, b: Degrees) -> f64 {
        let turn = (b.value() - a.value()).rem_euclid(360.0);
        if turn > 180.0 { turn - 360.0 } else { turn }
    }

    fn frankfurt_25c() -> Result<(LatLon, Degrees)> {
        Ok((LatLon::new(50.0325, 8.5866)?, Degrees::new(249.0)?))
    }

    #[test]
    fn test_climb_out_distance() {
        // 2,000 ft at 5 m/s while flying 90 m/s: 11 km beats 4 NM.
        let long = climb_out_distance(5.0, 90.0);
        assert!((long.value() - 609.6 / 5.0 * 90.0).abs() < 1.0);
        // A steep climber still flies 4 NM first.
        assert!((climb_out_distance(30.0, 70.0).value() - 7_408.0).abs() < 1.0);
    }

    #[test]
    fn test_departure_turn_point_lies_ahead_on_the_runway_heading() -> Result<()> {
        let (liftoff, heading) = frankfurt_25c()?;
        let point = departure_turn_point(liftoff, heading, Meters::new(9_000.0)?);
        assert!((liftoff.distance_to(point).value() - 9_000.0).abs() < 1.0);
        assert!(turn(liftoff.initial_bearing_to(point), heading).abs() < 0.1);
        Ok(())
    }

    #[test]
    fn test_arrival_from_behind_joins_straight_in() -> Result<()> {
        let (threshold, course) = frankfurt_25c()?;
        let far_behind = threshold.destination(Degrees::new(69.0)?, Meters::new(150_000.0)?);
        let points = arrival_points(far_behind, threshold, course);
        assert_eq!(points.len(), 2, "intercept point and final fix");
        let fix = threshold.destination(Degrees::new(69.0)?, Meters::new(FINAL)?);
        assert!(points[1].distance_to(fix).value() < 1.0);
        Ok(())
    }

    #[test]
    fn test_arrival_from_ahead_flies_downwind_and_base() -> Result<()> {
        let (threshold, course) = frankfurt_25c()?;
        // Coming from the west, the runway's far side.
        let ahead = threshold.destination(Degrees::new(260.0)?, Meters::new(150_000.0)?);
        let points = arrival_points(ahead, threshold, course);
        assert_eq!(points.len(), 4, "downwind, base, intercept, final fix");
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_arrivals_never_turn_more_than_a_right_angle(
            bearing in 0.0_f64..360.0,
            distance in 40_000.0_f64..600_000.0,
            course in 0.0_f64..360.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let threshold = LatLon::new(50.0, 8.5).map_err(fail)?;
            let course = Degrees::new(course).map_err(fail)?;
            let from = threshold.destination(
                Degrees::new(bearing).map_err(fail)?,
                Meters::new(distance).map_err(fail)?,
            );
            let mut path = vec![from];
            path.extend(arrival_points(from, threshold, course));
            path.push(threshold);
            for window in path.windows(3) {
                let incoming = Degrees::new(window[1].initial_bearing_to(window[0]).value() + 180.0)
                    .map_err(fail)?
                    .normalized();
                let outgoing = window[1].initial_bearing_to(window[2]);
                prop_assert!(turn(incoming, outgoing).abs() <= 100.0, "{:?}", path);
            }
            // The last leg into the threshold is the final course.
            let fix = path[path.len() - 2];
            prop_assert!(turn(fix.initial_bearing_to(threshold), course).abs() < 0.5);
        }
    }
}
