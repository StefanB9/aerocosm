//! Runway use by rule (plan M5 D113): the runways aligned with the
//! prevailing wind, how far apart they are, and what that lets an airport
//! handle.

use crate::{
    traffic::{Capacity, RunwayMode, operations::angle_between},
    units::Degrees,
    world::{Runway, RunwayDesignator, Surface},
};

/// Runways within this many degrees of each other are parallel.
const PARALLEL: f64 = 10.0;
/// Parallels at least this far apart run independently (ICAO Doc 9643:
/// independent parallel approaches); closer ones pair up, arrivals on one
/// and departures on the other, m.
const INDEPENDENT: f64 = 1_035.0;
/// Runways this far off the prevailing wind, degrees, still take it: a
/// climatological mean wind across them is a light crosswind.
const CROSSWIND: f64 = 60.0;
/// Movements per hour of a lane with one runway (mixed mode) or two
/// (segregated: arrivals on one, departures on the other).
const SINGLE_LANE: u16 = 40;
const DOUBLE_LANE: u16 = 60;
/// Short windows of ten minutes hold a sixth of the hour, rounded up.
const WINDOW_MINUTES: u16 = 10;

/// What the runways allow: the modes for both flows along the parallel
/// runways that handle most movements within [`CROSSWIND`] of the wind
/// (the best aligned among equals, or if none is that close), and the
/// movements per hour they handle.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunwayUse {
    pub(super) modes: Vec<RunwayMode>,
    pub(super) capacity: Capacity,
}

/// Runway use at an airport with `runways` and the prevailing wind `wind`,
/// or `None` without a runway.
pub(super) fn runway_use(runways: &[Runway], wind: Degrees) -> Option<RunwayUse> {
    let paved: Vec<&Runway> = runways
        .iter()
        .filter(|r| r.surface() == Surface::Paved)
        .collect();
    let usable: Vec<&Runway> = if paved.is_empty() {
        runways.iter().collect()
    } else {
        paved
    };
    // Parallel groups within reach of the wind: the one handling most
    // movements leads, the best aligned among equals.
    let mut best: Option<(Degrees, Vec<Vec<&Runway>>, u16, f64)> = None;
    let mut seen: Vec<Degrees> = Vec::new();
    for runway in &usable {
        let axis = runway.ends()[0].true_heading();
        if seen.iter().any(|s| axial_angle(*s, axis) <= PARALLEL) {
            continue;
        }
        seen.push(axis);
        let group: Vec<&Runway> = usable
            .iter()
            .filter(|r| axial_angle(r.ends()[0].true_heading(), axis) <= PARALLEL)
            .copied()
            .collect();
        let off = group
            .iter()
            .map(|r| misalignment(r, wind))
            .fold(f64::INFINITY, f64::min);
        let lanes = lanes(&group, axis);
        let per_hour = lanes
            .iter()
            .map(|lane| {
                if lane.len() >= 2 {
                    DOUBLE_LANE
                } else {
                    SINGLE_LANE
                }
            })
            .sum::<u16>();
        let better = match &best {
            None => true,
            Some((_, _, most, least_off)) => {
                let reachable = off <= CROSSWIND;
                let best_reachable = *least_off <= CROSSWIND;
                match (reachable, best_reachable) {
                    (true, false) => true,
                    (false, true) => false,
                    (true, true) => per_hour > *most || (per_hour == *most && off < *least_off),
                    (false, false) => off < *least_off,
                }
            }
        };
        if better {
            best = Some((axis, lanes, per_hour, off));
        }
    }
    let (axis, lanes, per_hour, _) = best?;
    let capacity = Capacity {
        per_hour,
        window_minutes: WINDOW_MINUTES,
        per_window: per_hour.div_ceil(60 / WINDOW_MINUTES),
    };
    let modes = (0..2)
        .map(|end| mode(&lanes, axis, end))
        .collect::<Option<Vec<_>>>()?;
    Some(RunwayUse { modes, capacity })
}

/// The mode for the flow along end `end` (0 or 1) of the primary runway's
/// axis: in a lane of two, arrivals on the first runway and departures on
/// the second; a lane of one runs mixed.
fn mode(lanes: &[Vec<&Runway>], axis: Degrees, end: usize) -> Option<RunwayMode> {
    let heading = if end == 0 {
        axis
    } else {
        Degrees::from_raw((axis.value() + 180.0).rem_euclid(360.0))
    };
    let designator = |runway: &Runway| -> RunwayDesignator {
        let [a, b] = runway.ends();
        if angle_between(a.true_heading(), heading) <= angle_between(b.true_heading(), heading) {
            a.designator()
        } else {
            b.designator()
        }
    };
    let (mut arrivals, mut departures) = (Vec::new(), Vec::new());
    for lane in lanes {
        match lane.as_slice() {
            [single] => {
                arrivals.push(designator(single));
                departures.push(designator(single));
            }
            [first, second, ..] => {
                arrivals.push(designator(first));
                departures.push(designator(second));
            }
            [] => {}
        }
    }
    let flow = Degrees::new(heading.value().rem_euclid(360.0) % 360.0).ok()?;
    (!arrivals.is_empty()).then_some(RunwayMode {
        flow,
        arrivals,
        departures,
    })
}

/// Parallel runways grouped into lanes that run independently of each
/// other: across the axis, a runway closer than [`INDEPENDENT`] to a lane's
/// first runway pairs up with it, one further away starts a new lane. Lanes
/// hold at most two runways; a third close one is not used.
fn lanes<'a>(group: &[&'a Runway], axis: Degrees) -> Vec<Vec<&'a Runway>> {
    let Some(origin) = group.first().map(|r| midpoint(r)) else {
        return Vec::new();
    };
    let mut across: Vec<(f64, &Runway)> = group
        .iter()
        .map(|r| {
            let to = midpoint(r);
            let offset = origin.distance_to(to).value()
                * (to_radians(origin.initial_bearing_to(to)) - to_radians(axis)).sin();
            (offset, *r)
        })
        .collect();
    across.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lanes: Vec<(f64, Vec<&Runway>)> = Vec::new();
    for (offset, runway) in across {
        match lanes.last_mut() {
            Some((start, lane)) if offset - *start < INDEPENDENT => {
                if lane.len() == 1 {
                    lane.push(runway);
                }
            }
            _ => lanes.push((offset, vec![runway])),
        }
    }
    lanes.into_iter().map(|(_, lane)| lane).collect()
}

fn midpoint(runway: &Runway) -> crate::LatLon {
    let [a, b] = runway.ends();
    let (from, to) = (a.threshold(), b.threshold());
    from.destination(
        from.initial_bearing_to(to),
        crate::units::Meters::from_raw(from.distance_to(to).value() / 2.0),
    )
}

fn to_radians(angle: Degrees) -> f64 {
    angle.value().to_radians()
}

/// The angle between two runway axes, `0..=90` degrees.
fn axial_angle(a: Degrees, b: Degrees) -> f64 {
    let angle = angle_between(a, b);
    angle.min(180.0 - angle)
}

/// How far `runway`'s axis lies off the wind, `0..=90` degrees.
fn misalignment(runway: &Runway, wind: Degrees) -> f64 {
    axial_angle(runway.ends()[0].true_heading(), wind)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        Id, LatLon,
        units::Meters,
        world::{GeometrySource, RunwayDefinition, RunwayEnd},
    };

    /// A runway along `heading` whose midpoint lies `across` metres to the
    /// right of a reference point near Frankfurt.
    fn runway(name: (&str, &str), heading: f64, across: f64, length: f64) -> Result<Runway> {
        let reference = LatLon::new(50.03, 8.55)?;
        let middle = reference.destination(
            Degrees::from_raw((heading + 90.0).rem_euclid(360.0)),
            Meters::from_raw(across.abs()),
        );
        let middle = if across < 0.0 {
            reference.destination(
                Degrees::from_raw((heading - 90.0).rem_euclid(360.0)),
                Meters::from_raw(-across),
            )
        } else {
            middle
        };
        let half = Meters::from_raw(length / 2.0);
        let low = middle.destination(Degrees::from_raw((heading + 180.0) % 360.0), half);
        let high = middle.destination(Degrees::from_raw(heading), half);
        Runway::new(
            Id::from_usize(0)?,
            &RunwayDefinition {
                airport: "EDDF".parse()?,
                ends: [
                    RunwayEnd::new(
                        name.0.parse()?,
                        low,
                        Degrees::new(heading)?,
                        Meters::new(0.0)?,
                    )?,
                    RunwayEnd::new(
                        name.1.parse()?,
                        high,
                        Degrees::new((heading + 180.0) % 360.0)?,
                        Meters::new(0.0)?,
                    )?,
                ],
                length: Meters::new(length)?,
                width: Meters::new(45.0)?,
                surface: Surface::Paved,
                geometry: GeometrySource::Surveyed,
            },
        )
    }

    fn names(list: &[RunwayDesignator]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn test_a_single_runway_runs_mixed_both_ways() -> Result<()> {
        let runways = [runway(("09", "27"), 90.0, 0.0, 3000.0)?];
        let used = runway_use(&runways, Degrees::new(250.0)?).context("a runway")?;
        assert_eq!(used.capacity.per_hour, 40);
        assert_eq!(used.capacity.per_window, 7);
        assert_eq!(used.modes.len(), 2);
        let west =
            crate::traffic::mode_for_wind(&used.modes, Degrees::new(250.0)?).context("a mode")?;
        assert_eq!(names(&west.arrivals), ["27"]);
        assert_eq!(names(&west.departures), ["27"]);
        Ok(())
    }

    #[test]
    fn test_parallels_by_spacing() -> Result<()> {
        let wind = Degrees::new(250.0)?;
        // 1,500 m apart: independent, both mixed.
        let independent = [
            runway(("09L", "27R"), 90.0, -750.0, 4000.0)?,
            runway(("09R", "27L"), 90.0, 750.0, 4000.0)?,
        ];
        let used = runway_use(&independent, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 80);
        let west = crate::traffic::mode_for_wind(&used.modes, wind).context("mode")?;
        assert_eq!(names(&west.arrivals), names(&west.departures));
        assert_eq!(west.arrivals.len(), 2);
        // 900 m apart: segregated.
        let segregated = [
            runway(("09L", "27R"), 90.0, -450.0, 4000.0)?,
            runway(("09R", "27L"), 90.0, 450.0, 4000.0)?,
        ];
        let used = runway_use(&segregated, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 60);
        let west = crate::traffic::mode_for_wind(&used.modes, wind).context("mode")?;
        assert_eq!((west.arrivals.len(), west.departures.len()), (1, 1));
        assert_ne!(west.arrivals, west.departures);
        // Three close parallels: the third is not used.
        let close = [
            runway(("09L", "27R"), 90.0, -300.0, 2500.0)?,
            runway(("09C", "27C"), 90.0, 0.0, 2500.0)?,
            runway(("09R", "27L"), 90.0, 300.0, 3500.0)?,
        ];
        let used = runway_use(&close, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 60);
        Ok(())
    }

    #[test]
    fn test_more_runways_a_little_off_the_wind_win() -> Result<()> {
        // Schiphol's pattern: a single runway into the wind, three
        // independent parallels 50 degrees off it.
        let runways = [
            runway(("06", "24"), 60.0, 3_000.0, 3500.0)?,
            runway(("36R", "18L"), 0.0, 1_500.0, 3400.0)?,
            runway(("36C", "18C"), 0.0, 0.0, 3300.0)?,
            runway(("36L", "18R"), 0.0, -1_500.0, 3800.0)?,
        ];
        let wind = Degrees::new(230.0)?;
        let used = runway_use(&runways, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 120);
        let mode = crate::traffic::mode_for_wind(&used.modes, wind).context("mode")?;
        assert!(names(&mode.arrivals).iter().all(|n| n.starts_with("18")));
        // Beyond 60 degrees a group is out of the wind's reach.
        let used = runway_use(&runways, Degrees::new(270.0)?).context("runways")?;
        assert_eq!(used.capacity.per_hour, 40);
        Ok(())
    }

    #[test]
    fn test_the_runway_best_aligned_with_the_wind_leads() -> Result<()> {
        // A layout like Frankfurt's: two close parallels, a third 1,400 m off
        // and a crossing runway, wind from the south-west.
        let runways = [
            runway(("07L", "25R"), 70.0, -1_918.0, 2800.0)?,
            runway(("07C", "25C"), 70.0, -1_400.0, 4000.0)?,
            runway(("07R", "25L"), 70.0, 0.0, 4000.0)?,
            runway(("18", "36"), 180.0, 1_000.0, 4000.0)?,
        ];
        let used = runway_use(&runways, Degrees::new(232.0)?).context("runways")?;
        assert_eq!(used.capacity.per_hour, 100, "a pair and a single lane");
        let west =
            crate::traffic::mode_for_wind(&used.modes, Degrees::new(232.0)?).context("mode")?;
        assert!((west.flow.value() - 250.0).abs() < 1e-6, "{:?}", west.flow);
        let all: Vec<String> = names(&west.arrivals)
            .into_iter()
            .chain(names(&west.departures))
            .collect();
        assert!(all.iter().all(|n| n.starts_with("25")), "{all:?}");
        // A wind from just west of north, 75 degrees off the parallels,
        // turns to the crossing runway.
        let north = runway_use(&runways, Degrees::new(355.0)?).context("runways")?;
        let mode =
            crate::traffic::mode_for_wind(&north.modes, Degrees::new(355.0)?).context("mode")?;
        assert_eq!(names(&mode.arrivals), ["36"]);
        assert!(runway_use(&[], Degrees::new(10.0)?).is_none());
        Ok(())
    }
}
