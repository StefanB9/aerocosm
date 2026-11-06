//! Holding patterns (plan M3 D57): standard right-hand racetracks at a fix,
//! one-minute legs and rate-one turns.

use crate::{
    LatLon,
    units::{Degrees, Meters},
};

/// Holding speed, m/s (230 kt).
pub(crate) const HOLD_SPEED: f64 = 118.3;
/// Rate-one turn: 3° per second.
const TURN_RATE: f64 = 3.0;
/// Seconds for each 180° turn and each straight leg.
const LEG: f64 = 60.0;
/// One full circuit, s.
pub(crate) const CYCLE: f64 = 4.0 * LEG;
/// Radius of a rate-one turn at holding speed, m.
pub(crate) const TURN_RADIUS: f64 = HOLD_SPEED / (TURN_RATE * std::f64::consts::PI / 180.0);

/// Points sampled around a pattern by [`holding_pattern`].
pub const PATTERN_POINTS: usize = 48;

/// The racetrack flown at `fix` on `inbound`, as a closed polyline starting
/// and ending at the fix, for drawing.
pub fn holding_pattern(fix: LatLon, inbound: Degrees) -> Vec<LatLon> {
    let mut points = Vec::with_capacity(PATTERN_POINTS + 1);
    for step in 0..=PATTERN_POINTS {
        // A few dozen points; the conversion is exact.
        #[allow(clippy::cast_precision_loss)]
        let elapsed = CYCLE * step as f64 / PATTERN_POINTS as f64;
        points.push(racetrack(fix, inbound, elapsed.min(CYCLE - 1e-9)).0);
    }
    points
}

fn course(value: f64) -> Degrees {
    Degrees::from_raw(value).normalized()
}

/// Position and course `elapsed` seconds after crossing `fix` on the
/// `inbound` course: right turn, outbound leg, right turn, inbound leg.
pub(crate) fn racetrack(fix: LatLon, inbound: Degrees, elapsed: f64) -> (LatLon, Degrees) {
    let t = elapsed.rem_euclid(CYCLE);
    let inbound = inbound.value();
    let radius = Meters::from_raw(TURN_RADIUS);
    let leg = Meters::from_raw(HOLD_SPEED * LEG);
    if t < LEG {
        // Turning right about a centre abeam the fix.
        let center = fix.destination(course(inbound + 90.0), radius);
        let radial = inbound - 90.0 + TURN_RATE * t;
        return (
            center.destination(course(radial), radius),
            course(radial + 90.0),
        );
    }
    let behind = fix.destination(course(inbound + 180.0), leg);
    let abeam = fix.destination(course(inbound + 90.0), Meters::from_raw(2.0 * TURN_RADIUS));
    let far_abeam = behind.destination(course(inbound + 90.0), Meters::from_raw(2.0 * TURN_RADIUS));
    // Straight legs run between the turns' exact end points, so the pattern
    // stays closed on the sphere.
    let straight = |from: LatLon, to: LatLon, seconds: f64| {
        from.interpolate(to, (seconds / LEG).clamp(0.0, 1.0))
            .unwrap_or(from)
    };
    if t < 2.0 * LEG {
        return (straight(abeam, far_abeam, t - LEG), course(inbound + 180.0));
    }
    if t < 3.0 * LEG {
        let center = behind.destination(course(inbound + 90.0), radius);
        let radial = inbound + 90.0 + TURN_RATE * (t - 2.0 * LEG);
        return (
            center.destination(course(radial), radius),
            course(radial + 90.0),
        );
    }
    (straight(behind, fix, t - 3.0 * LEG), course(inbound))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    fn fix() -> Result<LatLon> {
        LatLon::new(50.3, 8.9)
    }

    #[test]
    fn test_racetrack_starts_and_ends_at_the_fix_inbound() -> Result<()> {
        let inbound = Degrees::new(250.0)?;
        for elapsed in [0.0, CYCLE, 3.0 * CYCLE] {
            let (point, course) = racetrack(fix()?, inbound, elapsed);
            assert!(point.distance_to(fix()?).value() < 1.0, "at {elapsed} s");
            assert!((course.value() - 250.0).abs() < 1.0 || (course.value() - 250.0).abs() > 359.0);
        }
        // Half way round it flies outbound, two turn radii to the right.
        let (outbound, course) = racetrack(fix()?, inbound, 90.0);
        assert!((course.value() - 70.0).abs() < 1.0, "{course}");
        let offset = fix()?.distance_to(outbound).value();
        assert!(offset > 2.0 * TURN_RADIUS, "{offset}");
        Ok(())
    }

    #[test]
    fn test_holding_pattern_is_a_closed_loop_through_the_fix() -> Result<()> {
        let points = holding_pattern(fix()?, Degrees::new(90.0)?);
        assert_eq!(points.len(), PATTERN_POINTS + 1);
        let (first, last) = (points[0], points[PATTERN_POINTS]);
        assert!(first.distance_to(fix()?).value() < 1.0);
        assert!(last.distance_to(first).value() < 1.0);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_racetrack_is_continuous_and_bounded(
            inbound in 0.0_f64..360.0,
            start in 0.0_f64..1_000.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let fix = fix().map_err(fail)?;
            let inbound = Degrees::new(inbound).map_err(fail)?;
            let (mut previous, _) = racetrack(fix, inbound, start);
            for step in 1..=60 {
                let (here, _) = racetrack(fix, inbound, start + f64::from(step));
                // One second at holding speed.
                prop_assert!(previous.distance_to(here).value() <= HOLD_SPEED + 1.0);
                prop_assert!(fix.distance_to(here).value() <= HOLD_SPEED * 60.0 + 2.0 * TURN_RADIUS + 10.0);
                previous = here;
            }
        }
    }
}
