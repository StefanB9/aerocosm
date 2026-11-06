//! Runway entries and exits for a leg, and each mapped airport's typical
//! taxi times for schedules (plan M4 D71, D73).

use std::{cmp::Ordering, collections::BinaryHeap};

use super::{AirportGround, EdgeKind, Exit, GroundNetwork, Node};
use crate::{
    Id, SimDuration,
    ops::active_runways_now,
    traffic::TrafficTargets,
    units::Meters,
    world::{Airport, RunwayDesignator, World},
};

/// Average taxi speed along a route, m/s (about 10 kt with turns and slow
/// apron sections; unimpeded hub taxi-out times are around 10 minutes).
pub const TAXI_SPEED: f64 = 5.0;
/// Pushback and engine start before taxiing, s.
pub const PUSHBACK_TIME: f64 = 150.0;
/// Landing roll assumed for typical taxi-in times, m (an A320).
const TYPICAL_ROLL: f64 = 1_400.0;
/// Wingspan assumed for typical taxi times, m (an A320).
const TYPICAL_WINGSPAN: f64 = 36.0;
/// Runways cost this many times their length, as in routes.
const RUNWAY_PENALTY: f64 = 4.0;

/// Exits up to this angle off the landing direction are rapid exits,
/// taken at higher speed.
pub const RAPID_EXIT: f64 = 45.0;

impl AirportGround {
    /// Where departures from `runway` enter it: the entry nearest its
    /// threshold, for a full-length take-off.
    pub fn departure_entry(&self, runway: RunwayDesignator) -> Option<&Exit> {
        self.exits()
            .iter()
            .filter(|exit| exit.runway() == runway)
            .min_by(|a, b| a.distance().value().total_cmp(&b.distance().value()))
    }

    /// The first exit from `runway` a landing can make: a rapid exit after
    /// `rapid_roll`, any other after `roll`; the last exit if none is far
    /// enough.
    pub fn landing_exit(
        &self,
        runway: RunwayDesignator,
        rapid_roll: Meters,
        roll: Meters,
    ) -> Option<&Exit> {
        let mut on_runway: Vec<&Exit> = self
            .exits()
            .iter()
            .filter(|exit| exit.runway() == runway)
            .collect();
        on_runway.sort_by(|a, b| a.distance().value().total_cmp(&b.distance().value()));
        on_runway
            .iter()
            .find(|exit| {
                let needed = if exit.angle().value() <= RAPID_EXIT {
                    rapid_roll
                } else {
                    roll
                };
                exit.distance() >= needed
            })
            .or_else(|| on_runway.last())
            .copied()
    }

    /// Route length from `node` to every node (`towards` false) or from
    /// every node to `node` (`towards` true), for an aircraft of `wingspan`;
    /// infinite where unreachable. Runways are avoided as in routes.
    pub fn lengths_from(&self, node: Id<Node>, wingspan: Meters, towards: bool) -> Vec<f64> {
        let count = self.nodes().len();
        let mut cost = vec![f64::INFINITY; count];
        let mut length = vec![f64::INFINITY; count];
        let Some(start) = (node.as_usize() < count).then_some(node.as_usize()) else {
            return length;
        };
        cost[start] = 0.0;
        length[start] = 0.0;
        let mut heap = BinaryHeap::with_capacity(64);
        heap.push(Visit {
            cost: 0.0,
            node: start,
        });
        while let Some(Visit {
            cost: reached,
            node,
        }) = heap.pop()
        {
            if reached > cost[node] {
                continue;
            }
            for &edge_id in self.edges_at(Id::new(u32::try_from(node).unwrap_or(0))) {
                let Some(edge) = self.edge(edge_id) else {
                    continue;
                };
                let from_here = edge.from().as_usize() == node;
                // Searching backwards, an edge is usable when it can be
                // taxied towards this node.
                let usable = !edge.oneway() || from_here != towards;
                if !usable || edge.max_wingspan().is_some_and(|limit| limit < wingspan) {
                    continue;
                }
                let next = if from_here { edge.to() } else { edge.from() }.as_usize();
                let factor = if edge.kind() == EdgeKind::Runway {
                    RUNWAY_PENALTY
                } else {
                    1.0
                };
                let step = edge.length().value();
                if reached + step * factor < cost[next] {
                    cost[next] = reached + step * factor;
                    length[next] = length[node] + step;
                    heap.push(Visit {
                        cost: cost[next],
                        node: next,
                    });
                }
            }
        }
        length
    }

    /// Typical taxi-out and taxi-in times with the runway ends in use:
    /// pushback plus the median route from a stand to the nearest departure
    /// entry, and the median route from the nearest arrival exit to a stand.
    pub fn typical_taxi(
        &self,
        departures: &[RunwayDesignator],
        arrivals: &[RunwayDesignator],
    ) -> Option<TaxiTimes> {
        let wingspan = Meters::from_raw(TYPICAL_WINGSPAN);
        let roll = Meters::from_raw(TYPICAL_ROLL);
        let nearest = |nodes: Vec<Id<Node>>, towards: bool| -> Option<f64> {
            let tables: Vec<Vec<f64>> = nodes
                .into_iter()
                .map(|node| self.lengths_from(node, wingspan, towards))
                .collect();
            let mut per_stand: Vec<f64> = self
                .stands()
                .iter()
                .map(|stand| {
                    tables
                        .iter()
                        .map(|table| table[stand.node().as_usize()])
                        .fold(f64::INFINITY, f64::min)
                })
                .filter(|length| length.is_finite())
                .collect();
            per_stand.sort_by(f64::total_cmp);
            per_stand.get(per_stand.len() / 2).copied()
        };
        let entries = departures
            .iter()
            .filter_map(|runway| self.departure_entry(*runway).map(Exit::node))
            .collect();
        let exits = arrivals
            .iter()
            .filter_map(|runway| self.landing_exit(*runway, roll, roll).map(Exit::node))
            .collect();
        let out = nearest(entries, true)?;
        let back = nearest(exits, false)?;
        Some(TaxiTimes {
            out: seconds(PUSHBACK_TIME + out / TAXI_SPEED),
            back: seconds(back / TAXI_SPEED),
        })
    }
}

/// Whole seconds of `value`.
fn seconds(value: f64) -> SimDuration {
    // SAFETY: taxi times are minutes, far inside `i32`.
    #[allow(clippy::cast_possible_truncation)]
    SimDuration::from_secs(value.round() as i32)
}

/// A Dijkstra queue entry, smallest cost first.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Visit {
    cost: f64,
    node: usize,
}

impl Eq for Visit {}

impl Ord for Visit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// An airport's typical taxi times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxiTimes {
    /// Off-block to the departure runway, pushback included.
    pub out: SimDuration,
    /// Runway exit to on-block.
    pub back: SimDuration,
}

impl GroundNetwork {
    /// Typical taxi times of every mapped airport with the runways in use
    /// now, indexed by airport; `None` where unmapped.
    pub fn taxi_times(
        &self,
        world: &World,
        traffic: Option<&TrafficTargets>,
    ) -> Vec<Option<TaxiTimes>> {
        let mut times = vec![None; world.airport_count()];
        for ground in self.airports() {
            let airport: Id<Airport> = ground.airport();
            let Some(place) = world.airport(airport) else {
                continue;
            };
            let modes = traffic.map_or(&[][..], |t| t.runway_modes(place.ident()));
            let runways = world.runways_of(airport);
            let active = active_runways_now(runways, modes);
            let designator = |use_: &crate::ops::ActiveRunway| {
                runways[use_.runway].ends()[use_.end].designator()
            };
            let departures: Vec<RunwayDesignator> = active
                .iter()
                .filter(|a| a.departures)
                .map(designator)
                .collect();
            let arrivals: Vec<RunwayDesignator> = active
                .iter()
                .filter(|a| a.arrivals)
                .map(designator)
                .collect();
            if let Some(slot) = times.get_mut(airport.as_usize()) {
                *slot = ground.typical_taxi(&departures, &arrivals);
            }
        }
        times
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{ground::definition::tests::sample, test_data::data};

    fn shipped() -> Result<(World, GroundNetwork, TrafficTargets)> {
        let data = data()?;
        let ground = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &data.world,
        )?;
        Ok((data.world, ground, data.traffic))
    }

    #[test]
    fn test_entries_and_exits_follow_distance_and_angle() -> Result<()> {
        let mut definition = sample()?;
        definition.nodes.extend([[50.034, 8.56], [50.035, 8.56]]);
        let exit = definition.exits[0].clone();
        let mut rapid = exit.clone();
        rapid.node = 3;
        rapid.distance = Meters::new(1_300.0)?;
        rapid.angle = crate::units::Degrees::new(30.0)?;
        let mut late = exit.clone();
        late.node = 4;
        late.distance = Meters::new(3_000.0)?;
        definition.exits.extend([rapid, late]);
        let ground = AirportGround::new(definition, &data()?.world)?;
        let runway: RunwayDesignator = "07C".parse()?;
        let entry = ground.departure_entry(runway).context("an entry")?;
        assert_eq!(entry.node(), Id::new(3));
        let m = Meters::new;
        // The rapid exit at 1,300 m suits a short roll; a long roll misses it
        // and the 90° exit at 2,400 m alike.
        let short = ground
            .landing_exit(runway, m(1_200.0)?, m(1_500.0)?)
            .context("exit")?;
        assert_eq!(short.node(), Id::new(3));
        let long = ground
            .landing_exit(runway, m(1_400.0)?, m(1_500.0)?)
            .context("exit")?;
        assert_eq!(long.node(), Id::new(2));
        let longest = ground
            .landing_exit(runway, m(5_000.0)?, m(5_000.0)?)
            .context("exit")?;
        assert_eq!(longest.node(), Id::new(4));
        assert!(ground.departure_entry("25C".parse()?).is_none());
        Ok(())
    }

    #[test]
    fn test_lengths_from_respect_direction() -> Result<()> {
        let mut definition = sample()?;
        definition.edges[1].oneway = true; // 1 → 2 only
        let ground = AirportGround::new(definition, &data()?.world)?;
        let span = Meters::new(36.0)?;
        let from_stand = ground.lengths_from(Id::new(0), span, false);
        assert!(from_stand[2].is_finite() && from_stand[2] > 200.0);
        let to_stand = ground.lengths_from(Id::new(0), span, true);
        // Node 2 cannot be left towards the stand on the one-way edge.
        assert!(to_stand[2].is_infinite());
        assert!(to_stand[1].is_finite());
        let back_from_two = ground.lengths_from(Id::new(2), span, true);
        assert!(back_from_two[0].is_finite());
        assert!(
            ground
                .lengths_from(Id::new(9), span, false)
                .iter()
                .all(|l| l.is_infinite())
        );
        Ok(())
    }

    #[test]
    fn test_typical_taxi_times_at_the_hubs_are_plausible() -> Result<()> {
        let (world, ground, traffic) = shipped()?;
        let times = ground.taxi_times(&world, Some(&traffic));
        for (ident, out, back) in [("EDDF", (8, 22), (3, 14)), ("EDDM", (6, 18), (3, 12))] {
            let airport = world.airport_by_ident(ident.parse()?).context("airport")?;
            let taxi = times[airport.as_usize()].context("mapped")?;
            let minutes = |d: SimDuration| d.as_millis() / 60_000;
            assert!(
                (out.0..=out.1).contains(&minutes(taxi.out)),
                "{ident} out {taxi:?}"
            );
            assert!(
                (back.0..=back.1).contains(&minutes(taxi.back)),
                "{ident} in {taxi:?}"
            );
        }
        let saarbruecken = world.airport_by_ident("EDDR".parse()?).context("EDDR")?;
        assert_eq!(times[saarbruecken.as_usize()], None);
        Ok(())
    }
}
