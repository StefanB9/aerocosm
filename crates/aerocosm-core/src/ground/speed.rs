//! Taxi speeds along a route (plan M6 D154): 15 m/s on straight taxiways,
//! 8 m/s on apron taxilanes and through gentle bends, 5 m/s in sharp turns
//! and on stand lead-ins. Aircraft speed up and brake at 1 m/s², braking in
//! time for each lower limit ahead and for where they have to stop.

use super::{AirportGround, EdgeKind, GroundRoute};
use crate::{LatLon, TICK};

/// Speed on straight taxiways and runways, m/s (about 30 kt).
pub const STRAIGHT: f64 = 15.0;
/// Speed on apron taxilanes and through gentle bends, m/s.
pub const SLOW: f64 = 8.0;
/// Speed in sharp turns and on stand lead-ins, m/s.
pub const TURN: f64 = 5.0;
/// How fast an aircraft speeds up, m/s².
pub const ACCELERATION: f64 = 1.0;
/// How hard an aircraft brakes, m/s².
pub const BRAKING: f64 = 1.0;
/// A turn is measured over this far either side of a point of the path, and
/// its speed holds that far either side, m.
const TURN_WINDOW: f64 = 25.0;
/// A turn of at least this much over the window is sharp…
const SHARP: f64 = 60.0;
/// …and of at least this much a gentle bend, degrees.
const GENTLE: f64 = 25.0;
/// Closer to where it stops than a tick at this speed covers, an aircraft
/// rolls up to it, m/s.
const CREEP: f64 = 1.0;

/// How far an aircraft at `speed` m/s needs to stop, m.
pub const fn braking_distance(speed: f64) -> f64 {
    speed * speed / (2.0 * BRAKING)
}

/// The speed limits along one taxi path: pieces, each from a distance along
/// the path to the next one's start, and where the path ends.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedLimits {
    from: Vec<f64>,
    limit: Vec<f64>,
    end: f64,
}

impl SpeedLimits {
    /// The limits along a path through `points` (distance 0 at the first),
    /// whose route edges start `offset` metres along it as `(length, kind)`,
    /// ending `end` metres along. Before the route and beyond its last
    /// edge, the nearest edge's limit holds.
    pub fn new(
        points: &[LatLon],
        offset: f64,
        edges: impl IntoIterator<Item = (f64, EdgeKind)>,
        end: f64,
    ) -> Self {
        // Each edge's limit over its stretch; the first holds from the
        // path's start, the last to its end.
        let mut spans: Vec<(f64, f64, f64)> = Vec::new();
        let mut at = offset;
        for (length, kind) in edges {
            spans.push((at, at + length, of_kind(kind)));
            at += length;
        }
        if let Some(first) = spans.first_mut() {
            first.0 = f64::NEG_INFINITY;
        }
        if let Some(last) = spans.last_mut() {
            last.1 = f64::INFINITY;
        }
        if spans.is_empty() {
            spans.push((f64::NEG_INFINITY, f64::INFINITY, SLOW));
        }
        let turns = turns(points);
        let mut cuts: Vec<f64> = spans
            .iter()
            .flat_map(|&(a, b, _)| [a, b])
            .chain(turns.iter().flat_map(|&(a, b, _)| [a, b]))
            .filter(|cut| cut.is_finite() && *cut > 0.0)
            .collect();
        cuts.push(0.0);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let mut from: Vec<f64> = Vec::with_capacity(cuts.len());
        let mut limit: Vec<f64> = Vec::with_capacity(cuts.len());
        for (k, &cut) in cuts.iter().enumerate() {
            let next = cuts.get(k + 1).copied().unwrap_or(cut + 1.0);
            let middle = (cut + next) / 2.0;
            let covers = |&&(a, b, _): &&(f64, f64, f64)| a <= middle && middle < b;
            let here = spans
                .iter()
                .chain(turns.iter())
                .filter(covers)
                .map(|&(_, _, l)| l)
                .fold(STRAIGHT, f64::min);
            if limit.last().is_some_and(|&last| (last - here).abs() < 1e-9) {
                continue;
            }
            from.push(cut);
            limit.push(here);
        }
        Self { from, limit, end }
    }

    /// The piece `along` the path lies in.
    fn piece(&self, along: f64) -> usize {
        self.from.partition_point(|&f| f <= along).saturating_sub(1)
    }

    /// The limit `along` the path, m/s.
    pub fn limit_at(&self, along: f64) -> f64 {
        self.limit
            .get(self.piece(along))
            .copied()
            .unwrap_or(STRAIGHT)
    }

    /// The speed for the next tick of an aircraft `along` the path at
    /// `speed` m/s that has to stop `stop` metres along it, m/s: speeding
    /// up, keeping to the limit and braking in time for lower ones ahead.
    pub fn next_speed(&self, along: f64, speed: f64, stop: f64) -> f64 {
        let dt = TICK.as_secs_f64();
        let mut next = (speed + ACCELERATION * dt).min(self.limit_at(along));
        let horizon = along + braking_distance(next) + next * dt + 1.0;
        for (&from, &limit) in self
            .from
            .iter()
            .zip(&self.limit)
            .skip(self.piece(along) + 1)
            .take_while(|&(&from, _)| from <= horizon)
        {
            next = next.min(reach(from - along, limit));
        }
        let to_stop = stop - along;
        if to_stop <= 0.0 {
            return 0.0;
        }
        // Rolling up the last metre, not ever closer by halves.
        let to_stand = reach(to_stop, 0.0).max((to_stop / dt).min(CREEP));
        next.min(to_stand).max(0.0)
    }

    /// How long the path from `along` takes unimpeded, starting at `speed`
    /// m/s and stopping at its end, ms.
    pub fn unimpeded(&self, along: f64, speed: f64) -> i64 {
        self.time_to(along, speed, self.end)
    }

    /// How long the path from `along` to `to` takes unimpeded, starting at
    /// `speed` m/s and stopping there, ms.
    pub fn time_to(&self, along: f64, speed: f64, to: f64) -> i64 {
        let dt = TICK.as_secs_f64();
        // SAFETY: a few kilometres at creeping speed at least, far inside
        // `i64`.
        #[allow(clippy::cast_possible_truncation)]
        let most = ((to - along).max(0.0) / (CREEP * dt)) as i64 + 10;
        let (mut along, mut speed, mut ticks) = (along, speed, 0);
        while along < to - 1e-6 && ticks < most {
            speed = self.next_speed(along, speed, to);
            along = (along + speed * dt).min(to);
            ticks += 1;
        }
        ticks * TICK.as_millis()
    }
}

impl AirportGround {
    /// How long `route` takes to taxi unimpeded, from standing to standing,
    /// s.
    pub fn taxi_time(&self, route: &GroundRoute) -> f64 {
        let length = route.length().value();
        let limits = SpeedLimits::new(
            route.points(),
            0.0,
            route
                .edges()
                .iter()
                .filter_map(|&(edge, _)| self.edge(edge).map(|e| (e.length().value(), e.kind()))),
            length,
        );
        // SAFETY: minutes in ms, exact in `f64`.
        #[allow(clippy::cast_precision_loss)]
        let seconds = limits.unimpeded(0.0, 0.0) as f64 / 1_000.0;
        seconds
    }
}

/// The limit on an edge of `kind`, m/s.
const fn of_kind(kind: EdgeKind) -> f64 {
    match kind {
        EdgeKind::Taxiway | EdgeKind::Runway => STRAIGHT,
        EdgeKind::Taxilane => SLOW,
        EdgeKind::LeadIn => TURN,
    }
}

/// The fastest an aircraft may go now to be down to `limit` m/s `ahead`
/// metres on, moving one tick at that speed first, m/s.
fn reach(ahead: f64, limit: f64) -> f64 {
    let b = BRAKING * TICK.as_secs_f64();
    -b + (b * b + limit * limit + 2.0 * BRAKING * ahead.max(0.0)).sqrt()
}

/// The turns along `points`, as stretches `(from, to, limit)` around each
/// point where the path turns gently or sharply.
fn turns(points: &[LatLon]) -> Vec<(f64, f64, f64)> {
    let mut along = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (i, point) in points.iter().enumerate() {
        if let Some(previous) = i.checked_sub(1).and_then(|p| points.get(p)) {
            total += previous.distance_to(*point).value();
        }
        along.push(total);
    }
    let at = |distance: f64| -> Option<LatLon> {
        let k = along.partition_point(|&a| a < distance).max(1);
        let (a, b) = (points.get(k - 1)?, points.get(k)?);
        let (s, t) = (along[k - 1], along[k]);
        let fraction = if t > s { (distance - s) / (t - s) } else { 1.0 };
        a.interpolate(*b, fraction.clamp(0.0, 1.0)).ok()
    };
    let mut turns = Vec::new();
    for (i, point) in points.iter().enumerate().skip(1) {
        let s = along[i];
        if s >= total {
            break;
        }
        let (Some(before), Some(after)) = (
            at((s - TURN_WINDOW).max(0.0)),
            at((s + TURN_WINDOW).min(total)),
        ) else {
            continue;
        };
        if before.distance_to(*point).value() < 1.0 || point.distance_to(after).value() < 1.0 {
            continue;
        }
        let into = before.initial_bearing_to(*point).value();
        let out = point.initial_bearing_to(after).value();
        let turn = (out - into).rem_euclid(360.0);
        let turn = turn.min(360.0 - turn);
        let limit = if turn >= SHARP {
            TURN
        } else if turn >= GENTLE {
            SLOW
        } else {
            continue;
        };
        turns.push((s - TURN_WINDOW, s + TURN_WINDOW, limit));
    }
    turns
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::units::{Degrees, Meters};

    const DT: f64 = 1.0;

    /// A path from `start` heading `course`, through legs of
    /// `(length, turn after it)`.
    fn path(legs: &[(f64, f64)]) -> Result<Vec<LatLon>> {
        let mut here = LatLon::new(50.0, 8.5)?;
        let mut course = 90.0;
        let mut points = vec![here];
        for &(length, turn) in legs {
            here = here.destination(Degrees::from_raw(course), Meters::from_raw(length));
            points.push(here);
            course += turn;
        }
        Ok(points)
    }

    #[test]
    fn test_limits_follow_the_edge_kinds() -> Result<()> {
        let points = path(&[(1_000.0, 0.0)])?;
        let limits = SpeedLimits::new(
            &points,
            100.0,
            [
                (300.0, EdgeKind::Taxilane),
                (400.0, EdgeKind::Taxiway),
                (200.0, EdgeKind::LeadIn),
            ],
            1_000.0,
        );
        assert!(
            (limits.limit_at(50.0) - SLOW).abs() < 1e-9,
            "before the route"
        );
        assert!((limits.limit_at(250.0) - SLOW).abs() < 1e-9);
        assert!((limits.limit_at(600.0) - STRAIGHT).abs() < 1e-9);
        assert!((limits.limit_at(900.0) - TURN).abs() < 1e-9);
        assert!((limits.limit_at(1_200.0) - TURN).abs() < 1e-9, "beyond");
        Ok(())
    }

    #[test]
    fn test_turns_slow_the_taxi_down() -> Result<()> {
        // A right angle after 400 m, a gentle 30° bend after 800 m.
        let points = path(&[(400.0, 90.0), (400.0, 30.0), (400.0, 0.0)])?;
        let limits = SpeedLimits::new(&points, 0.0, [(1_200.0, EdgeKind::Taxiway)], 1_200.0);
        assert!((limits.limit_at(200.0) - STRAIGHT).abs() < 1e-9);
        assert!((limits.limit_at(390.0) - TURN).abs() < 1e-9);
        assert!((limits.limit_at(415.0) - TURN).abs() < 1e-9);
        assert!((limits.limit_at(600.0) - STRAIGHT).abs() < 1e-9);
        assert!((limits.limit_at(800.0) - SLOW).abs() < 1e-9);
        assert!((limits.limit_at(1_000.0) - STRAIGHT).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_an_aircraft_speeds_up_and_keeps_to_the_limit() -> Result<()> {
        let points = path(&[(2_000.0, 0.0)])?;
        let limits = SpeedLimits::new(&points, 0.0, [(2_000.0, EdgeKind::Taxiway)], 2_000.0);
        let speed = limits.next_speed(0.0, 0.0, 2_000.0);
        assert!((speed - ACCELERATION * DT).abs() < 1e-9);
        let speed = limits.next_speed(500.0, STRAIGHT, 2_000.0);
        assert!((speed - STRAIGHT).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_an_aircraft_brakes_in_time_for_a_stop_and_a_turn() -> Result<()> {
        // A sharp turn 1,000 m along, a stop at 1,500 m.
        let points = path(&[(1_000.0, 90.0), (1_000.0, 0.0)])?;
        let limits = SpeedLimits::new(&points, 0.0, [(2_000.0, EdgeKind::Taxiway)], 2_000.0);
        let (mut along, mut speed) = (0.0, 0.0);
        let mut fastest_in_turn: f64 = 0.0;
        let mut fastest: f64 = 0.0;
        for _ in 0..1_000 {
            let next = limits.next_speed(along, speed, 1_500.0);
            assert!(next <= speed + ACCELERATION * DT + 1e-9, "speeds up gently");
            assert!(
                next >= speed - 2.0 * BRAKING * DT,
                "brakes gently: {speed} → {next}"
            );
            speed = next;
            along = (along + speed * DT).min(1_500.0);
            fastest = fastest.max(speed);
            if (975.0..=1_025.0).contains(&along) {
                fastest_in_turn = fastest_in_turn.max(speed);
            }
        }
        assert!((along - 1_500.0).abs() < 1e-6, "reaches the stop: {along}");
        assert!(fastest_in_turn <= TURN + 1e-9, "{fastest_in_turn}");
        assert!((fastest - STRAIGHT).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_an_unimpeded_taxi_takes_its_time() -> Result<()> {
        let points = path(&[(1_500.0, 0.0)])?;
        let limits = SpeedLimits::new(&points, 0.0, [(1_500.0, EdgeKind::Taxiway)], 1_500.0);
        // 15 s up to speed (112.5 m), as long down again, 1,275 m at 15
        // m/s: 115 s, give or take the ticks rolling up to the end.
        let seconds = limits.unimpeded(0.0, 0.0) / 1_000;
        assert!((113..=125).contains(&seconds), "{seconds}");
        // Half way already: about half.
        let rest = limits.unimpeded(750.0, STRAIGHT) / 1_000;
        assert!((55..=70).contains(&rest), "{rest}");
        // From a stand to 100 m on: under 25 s (it was 20 s at 5 m/s).
        let first = limits.time_to(0.0, 0.0, 100.0) / 1_000;
        assert!((15..=25).contains(&first), "{first}");
        Ok(())
    }

    #[test]
    fn test_a_route_takes_its_time_by_its_kinds_and_turns() -> Result<()> {
        let world = crate::test_data::data()?.world;
        let network =
            crate::ground::GroundNetwork::from_definition(crate::test_data::ground()?, &world)?;
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let layout = network.airport(frankfurt).context("EDDF's ground")?;
        let (a, b) = (layout.stands()[0].node(), layout.stands()[40].node());
        let route = layout.route(a, b, Meters::new(36.0)?).context("route")?;
        let length = route.length().value();
        let seconds = layout.taxi_time(&route);
        assert!(length > 500.0, "{length}");
        assert!(
            seconds > length / STRAIGHT && seconds < length / TURN + 30.0,
            "{seconds} s for {length} m"
        );
        Ok(())
    }

    #[test]
    fn test_braking_distance_from_taxi_speed() {
        assert!((braking_distance(STRAIGHT) - 112.5).abs() < 1e-9);
        assert!(braking_distance(0.0).abs() < 1e-9);
    }
}
