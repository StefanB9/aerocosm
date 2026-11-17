//! Runway use by rule (plan M5 D113, M7 D158): the groups of runways an
//! airport has, how far apart and whether they cross, and what that lets it
//! handle; the prevailing wind sets which way they run.

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
/// Runways shorter than this do not count where an airport has a longer
/// one: general aviation strips beside a hub's runways, m.
const SHORT: f64 = 1_500.0;
/// Movements per hour of a lane with one runway (mixed mode) or two
/// (segregated: arrivals on one, departures on the other), calibrated
/// against published capacities (plan M7 D157, D158).
const SINGLE_LANE: f64 = 45.0;
const DOUBLE_LANE: f64 = 85.0;
/// Another group of parallels that crosses none in use adds this share of
/// its lanes: it shares the airspace and the taxiways…
const SECOND_GROUP: f64 = 0.4;
/// …and a runway group that crosses one in use takes departures in the
/// gaps, movements per hour.
const CROSSING: f64 = 15.0;
/// Short windows of ten minutes hold a sixth of the hour, rounded up.
const WINDOW_MINUTES: u16 = 10;

/// What the runways allow: the modes for both flows and the movements per
/// hour they handle.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunwayUse {
    pub(super) modes: Vec<RunwayMode>,
    pub(super) capacity: Capacity,
}

/// A group of parallel runways: its axis, lanes, movements an hour and how
/// far it lies off the wind.
struct Group<'a> {
    axis: Degrees,
    lanes: Vec<Vec<&'a Runway>>,
    per_hour: f64,
    off: f64,
}

/// How a group is used beside the leading one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Arrivals and departures, as its lanes allow.
    Both,
    /// Departures only, in the gaps of the runway it crosses.
    Departures,
}

/// Runway use at an airport with `runways` and the prevailing wind `wind`,
/// or `None` without a runway (plan M7 D158): the group of parallels that
/// handles most movements leads (the best aligned among equals); each other
/// group adds a share of its lanes where it crosses none in use, or
/// departures where it does. The wind decides only which way they run.
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
    let long: Vec<&Runway> = usable
        .iter()
        .copied()
        .filter(|r| r.length().value() >= SHORT)
        .collect();
    let usable = if long.is_empty() { usable } else { long };
    let mut groups: Vec<Group> = Vec::new();
    for runway in &usable {
        let axis = runway.ends()[0].true_heading();
        if groups.iter().any(|g| axial_angle(g.axis, axis) <= PARALLEL) {
            continue;
        }
        let members: Vec<&Runway> = usable
            .iter()
            .filter(|r| axial_angle(r.ends()[0].true_heading(), axis) <= PARALLEL)
            .copied()
            .collect();
        let off = members
            .iter()
            .map(|r| misalignment(r, wind))
            .fold(f64::INFINITY, f64::min);
        let lanes = lanes(&members, axis);
        let per_hour = lanes
            .iter()
            .map(|lane| {
                if lane.len() >= 2 {
                    DOUBLE_LANE
                } else {
                    SINGLE_LANE
                }
            })
            .sum();
        groups.push(Group {
            axis,
            lanes,
            per_hour,
            off,
        });
    }
    // Most movements first, the best aligned among equals.
    groups.sort_by(|a, b| {
        b.per_hour
            .total_cmp(&a.per_hour)
            .then(a.off.total_cmp(&b.off))
    });
    let (lead, others) = groups.split_first()?;
    let mut per_hour = lead.per_hour;
    let mut used: Vec<(&Group, Role)> = vec![(lead, Role::Both)];
    for group in others {
        let crosses = used.iter().any(|(other, _)| groups_cross(group, other));
        if crosses {
            per_hour += CROSSING;
            used.push((group, Role::Departures));
        } else {
            per_hour += SECOND_GROUP * group.per_hour;
            used.push((group, Role::Both));
        }
    }
    // SAFETY: a few hundred movements an hour at most.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let per_hour = per_hour.round().max(1.0) as u16;
    let capacity = Capacity {
        per_hour,
        window_minutes: WINDOW_MINUTES,
        per_window: per_hour.div_ceil(60 / WINDOW_MINUTES),
    };
    let modes = [
        lead.axis,
        Degrees::from_raw((lead.axis.value() + 180.0).rem_euclid(360.0)),
    ]
    .into_iter()
    .map(|heading| mode(&used, heading))
    .collect::<Option<Vec<_>>>()?;
    Some(RunwayUse { modes, capacity })
}

/// The mode for the flow along `heading`, the leading group's direction:
/// each group runs along its end nearest that heading; in a lane of two,
/// arrivals on the first runway and departures on the second; a lane of one
/// runs mixed; a crossing group takes departures on its longest runway.
fn mode(used: &[(&Group, Role)], heading: Degrees) -> Option<RunwayMode> {
    let designator = |runway: &Runway| -> RunwayDesignator {
        let [a, b] = runway.ends();
        if angle_between(a.true_heading(), heading) <= angle_between(b.true_heading(), heading) {
            a.designator()
        } else {
            b.designator()
        }
    };
    let (mut arrivals, mut departures) = (Vec::new(), Vec::new());
    for (group, role) in used {
        match role {
            Role::Both => {
                for lane in &group.lanes {
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
            }
            Role::Departures => {
                if let Some(longest) = group
                    .lanes
                    .iter()
                    .flatten()
                    .max_by(|a, b| a.length().value().total_cmp(&b.length().value()))
                {
                    departures.push(designator(longest));
                }
            }
        }
    }
    let flow = Degrees::new(heading.value().rem_euclid(360.0) % 360.0).ok()?;
    (!arrivals.is_empty()).then_some(RunwayMode {
        flow,
        arrivals,
        departures,
    })
}

/// Whether a runway of `a` crosses one of `b`.
fn groups_cross(a: &Group, b: &Group) -> bool {
    let all = |g: &Group| -> Vec<(crate::LatLon, crate::LatLon)> {
        g.lanes
            .iter()
            .flatten()
            .map(|r| (r.ends()[0].threshold(), r.ends()[1].threshold()))
            .collect()
    };
    let (first, second) = (all(a), all(b));
    let Some(origin) = first.first().map(|(p, _)| *p) else {
        return false;
    };
    let flat = |p: crate::LatLon| -> (f64, f64) {
        let d = origin.distance_to(p).value();
        let bearing = origin.initial_bearing_to(p).value().to_radians();
        (d * bearing.sin(), d * bearing.cos())
    };
    first.iter().any(|&(p, q)| {
        second
            .iter()
            .any(|&(r, t)| segments_cross(flat(p), flat(q), flat(r), flat(t)))
    })
}

/// Whether segment `p`–`q` crosses segment `r`–`t` in the plane.
fn segments_cross(p: (f64, f64), q: (f64, f64), r: (f64, f64), t: (f64, f64)) -> bool {
    let side = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    side(p, q, r) * side(p, q, t) < 0.0 && side(r, t, p) * side(r, t, q) < 0.0
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
        assert_eq!(used.capacity.per_hour, 45);
        assert_eq!(used.capacity.per_window, 8);
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
        assert_eq!(used.capacity.per_hour, 90);
        let west = crate::traffic::mode_for_wind(&used.modes, wind).context("mode")?;
        assert_eq!(names(&west.arrivals), names(&west.departures));
        assert_eq!(west.arrivals.len(), 2);
        // 900 m apart: segregated.
        let segregated = [
            runway(("09L", "27R"), 90.0, -450.0, 4000.0)?,
            runway(("09R", "27L"), 90.0, 450.0, 4000.0)?,
        ];
        let used = runway_use(&segregated, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 85);
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
        assert_eq!(used.capacity.per_hour, 85);
        // A short strip beside them does not count.
        let mut strip = close.to_vec();
        strip.push(runway(("18", "36"), 180.0, 2_500.0, 900.0)?);
        let used = runway_use(&strip, wind).context("runways")?;
        assert_eq!(used.capacity.per_hour, 85);
        Ok(())
    }

    #[test]
    fn test_more_runways_off_the_wind_win() -> Result<()> {
        // Schiphol's pattern: a single runway into the wind, three
        // independent parallels 50 degrees off it, crossing none.
        let runways = [
            runway(("06", "24"), 60.0, 3_000.0, 3500.0)?,
            runway(("36R", "18L"), 0.0, 1_500.0, 3400.0)?,
            runway(("36C", "18C"), 0.0, 0.0, 3300.0)?,
            runway(("36L", "18R"), 0.0, -1_500.0, 3800.0)?,
        ];
        let wind = Degrees::new(230.0)?;
        let used = runway_use(&runways, wind).context("runways")?;
        // Three lanes of one, and four tenths of the single runway.
        assert_eq!(used.capacity.per_hour, 153);
        let mode = crate::traffic::mode_for_wind(&used.modes, wind).context("mode")?;
        let arrivals = names(&mode.arrivals);
        assert_eq!(arrivals.len(), 4, "{arrivals:?}");
        assert_eq!(arrivals.iter().filter(|n| n.starts_with("18")).count(), 3);
        assert!(arrivals.contains(&"24".to_string()));
        // A wind across the parallels changes only which way they run
        // (plan M7 D158: the mean wind is light).
        let used = runway_use(&runways, Degrees::new(270.0)?).context("runways")?;
        assert_eq!(used.capacity.per_hour, 153);
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
        assert_eq!(
            used.capacity.per_hour, 145,
            "a pair, a single lane and departures across"
        );
        let west =
            crate::traffic::mode_for_wind(&used.modes, Degrees::new(232.0)?).context("mode")?;
        assert!((west.flow.value() - 250.0).abs() < 1e-6, "{:?}", west.flow);
        assert!(names(&west.arrivals).iter().all(|n| n.starts_with("25")));
        assert_eq!(names(&west.departures), ["25C", "25L", "18"]);
        // A wind from just west of north turns the flow, not the runways:
        // the crossing runway still takes departures only.
        let north = runway_use(&runways, Degrees::new(355.0)?).context("runways")?;
        let mode =
            crate::traffic::mode_for_wind(&north.modes, Degrees::new(355.0)?).context("mode")?;
        assert!(names(&mode.arrivals).iter().all(|n| n.starts_with("07")));
        assert_eq!(
            names(&mode.departures).last().map(String::as_str),
            Some("36")
        );
        assert!(runway_use(&[], Degrees::new(10.0)?).is_none());
        Ok(())
    }
}
