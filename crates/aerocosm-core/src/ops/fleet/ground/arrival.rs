//! Arrivals under ground control: stand or waiting spot at touchdown, and
//! the wait for a stand (plan M4 D69, D76).

use super::{FOLLOW, RETRY, SPOT_ESCAPE_AFTER, Start, Target, ms};
use crate::{
    Id, SimTime,
    geo::Polyline,
    ground::{AirportGround, Exit, Node},
    ops::fleet::Operations,
    scenario::Scenario,
    world::{Airport, codes::AirlineIcao},
};

/// A landing vacates where its way is clear this far from the exit node:
/// off the runway and past its hold, m.
const VACATE_CLEAR: f64 = 200.0;
/// …and nobody comes the other way this far, m: about the way from the
/// exit to the aprons. Beyond, it waits for the way like any taxi; a
/// landing cannot wait for its whole route on the runway.
const VACATE_REACH: f64 = 3_000.0;

impl Operations {
    /// At touchdown at a controlled airport: a free stand or a waiting spot,
    /// and the taxi from the exit to it. `false` if the airport is not
    /// controlled or no route exists.
    pub(in crate::ops::fleet) fn plan_arrival(
        &mut self,
        scenario: &Scenario,
        index: usize,
        destination: Id<Airport>,
        airline: AirlineIcao,
        now: SimTime,
    ) -> bool {
        if !self.ground.controls(destination) {
            return false;
        }
        let Some((exit, near)) = self.paths[index]
            .as_ref()
            .and_then(|path| path.exit().map(|exit| (exit, path.exit_point())))
        else {
            return false;
        };
        let Some(layout) = scenario.ground().airport(destination) else {
            return false;
        };
        let free = self.find_stand(
            scenario,
            index,
            destination,
            airline,
            (near, Some(exit)),
            false,
        );
        let spot = if free.is_none() {
            self.free_spot(scenario, index, destination, exit)
        } else {
            None
        };
        let stand = match (free, spot) {
            (Some(stand), _) => Some(stand),
            (None, Some(_)) => None,
            (None, None) => {
                self.take_stand(scenario, index, destination, airline, (near, Some(exit)))
            }
        };
        let (to, target) = match (stand, spot) {
            (Some(stand), _) => (
                layout.stands()[stand.as_usize()].node(),
                Target::Stand(stand),
            ),
            (None, Some(spot)) => (spot, Target::Spot(spot)),
            (None, None) => return false,
        };
        let heading = stand.map(|s| layout.stands()[s.as_usize()].heading());
        let Some((vacate, polyline)) =
            self.vacate(scenario, index, destination, exit, (to, target), now)
        else {
            return false;
        };
        if let Target::Spot(node) = target {
            self.stats.waited_for_stand(destination);
            if let Some(airside) = self.ground.airside_mut(destination)
                && let Some(slot) = airside.spots.iter().position(|s| *s == node)
            {
                airside.spot_owner[slot] = Some(Self::owner(index));
            }
        }
        if let Some(path) = self.paths[index].as_mut() {
            if vacate != exit {
                let point = layout.nodes()[vacate.as_usize()].position();
                if let Some(length) = path.set_exit(vacate, point) {
                    self.profiles[index].set_landing_length(length);
                }
            }
            let length = path.set_taxi_in(polyline, heading.unwrap_or_default());
            self.profiles[index].set_taxi_in_length(length);
        }
        true
    }

    /// Where a landing that planned to turn off at `planned` vacates the
    /// runway for `goal`: the first exit from there on whose way off the
    /// runway is clear, its taxiways locked at once so that it never stops
    /// on the runway; else the planned exit. Starts the taxi from it;
    /// returns the exit and the taxi's polyline.
    fn vacate(
        &mut self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        planned: Id<Node>,
        goal: (Id<Node>, Target),
        now: SimTime,
    ) -> Option<(Id<Node>, Polyline)> {
        let layout = scenario.ground().airport(airport)?;
        let owner = Self::owner(index);
        let (to, target) = goal;
        let exits = later_exits(layout, planned);
        let start = |route, from| Start {
            route,
            from,
            target,
            base: Vec::new(),
            offset: 0.0,
            extension: Vec::new(),
        };
        for &exit in &exits {
            let Some(route) = self.route_for(layout, index, exit, to) else {
                continue;
            };
            if let Some(polyline) =
                self.start_taxi(scenario, index, airport, start(route, exit), now)
                && self.clear_way(index, 0.0, VACATE_CLEAR, VACATE_REACH)
            {
                return Some((exit, polyline));
            }
            self.ground.release(airport, owner);
            self.taxis[index] = None;
        }
        // No way off is clear: it waits on the runway at the first exit
        // nobody stands near (else its planned one), and the next arrival
        // goes around (plan M4 D90).
        let empty = |exit: Id<Node>| {
            let point = layout.nodes()[exit.as_usize()].position();
            self.ground.airside(airport).is_none_or(|airside| {
                !airside
                    .seen
                    .iter()
                    .any(|seen| seen.tail != owner && seen.at.distance_to(point).value() < FOLLOW)
            })
        };
        let wait = exits
            .iter()
            .copied()
            .find(|&exit| empty(exit))
            .unwrap_or(planned);
        let route = self.route_for(layout, index, wait, to)?;
        let polyline = self.start_taxi(scenario, index, airport, start(route, wait), now)?;
        Some((wait, polyline))
    }

    /// The free waiting spot with the shortest taxi from `from` for tail
    /// `index`, among those from which a stand that fits it can be reached
    /// (a spot in a dead end would keep it there).
    pub(super) fn free_spot(
        &self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        from: Id<Node>,
    ) -> Option<Id<Node>> {
        let layout = scenario.ground().airport(airport)?;
        let airside = self.ground.airside(airport)?;
        let lengths = layout.lengths_from(from, self.wingspans[index], false);
        let length = |spot: Id<Node>| {
            lengths
                .get(spot.as_usize())
                .copied()
                .unwrap_or(f64::INFINITY)
        };
        let wingspan = self.wingspans[index];
        let leads_on = |spot: Id<Node>| {
            let reach = layout.reachable_from(spot, wingspan);
            layout.stands().iter().any(|stand| {
                stand.max_wingspan() >= wingspan
                    && reach.get(stand.node().as_usize()).copied().unwrap_or(false)
            })
        };
        let mut spots: Vec<Id<Node>> = airside
            .spots
            .iter()
            .zip(&airside.spot_owner)
            .filter(|&(&spot, owner)| owner.is_none() && length(spot).is_finite())
            .map(|(spot, _)| *spot)
            .collect();
        spots.sort_by(|a, b| length(*a).total_cmp(&length(*b)));
        spots.into_iter().find(|&spot| leads_on(spot))
    }

    /// A free waiting spot to board remote at (D127): the one nearest the
    /// airport's reference point.
    pub(in crate::ops::fleet) fn boarding_spot(
        &self,
        scenario: &Scenario,
        airport: Id<Airport>,
    ) -> Option<Id<Node>> {
        let layout = scenario.ground().airport(airport)?;
        let airside = self.ground.airside(airport)?;
        let at = scenario.world().airport(airport)?.location();
        airside
            .spots
            .iter()
            .zip(&airside.spot_owner)
            .filter(|(_, owner)| owner.is_none())
            .map(|(&spot, _)| {
                (
                    spot,
                    layout.nodes()[spot.as_usize()]
                        .position()
                        .distance_to(at)
                        .value(),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(spot, _)| spot)
    }

    /// At a waiting spot: takes a stand as soon as one is free and taxies
    /// there; after half an hour it deplanes there and is towed to remote
    /// parking (D127). Returns whether it did.
    pub(super) fn wait_for_stand(
        &mut self,
        scenario: &Scenario,
        index: usize,
        now: SimTime,
        airline: AirlineIcao,
    ) -> bool {
        let Some(taxi) = self.taxis[index].as_ref() else {
            return false;
        };
        if taxi.waiting.is_some_and(|retry| ms(now) < retry) {
            return false;
        }
        let (airport, offset) = (taxi.airport, taxi.end);
        let escape = ms(now) - taxi.spot_since >= SPOT_ESCAPE_AFTER;
        let Some(here) = taxi.end_node else {
            return false;
        };
        let Some(layout) = scenario.ground().airport(airport) else {
            return false;
        };
        let near = layout.nodes()[here.as_usize()].position();
        let stand = self.find_stand(scenario, index, airport, airline, (near, Some(here)), false);
        if stand.is_none() && escape {
            self.ground.release(airport, Self::owner(index));
            self.taxis[index] = None;
            self.parking[index] = super::super::remote::Parking::Remote(None);
            return true;
        }
        let route = stand.and_then(|stand| {
            self.route_for(
                layout,
                index,
                here,
                layout.stands()[stand.as_usize()].node(),
            )
        });
        let (Some(stand), Some(route)) = (stand, route) else {
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.waiting = Some(ms(now) + RETRY);
            }
            return false;
        };
        let (base, escaped) = self.taxis[index].as_ref().map_or_else(
            || (Vec::new(), false),
            |taxi| (taxi.points.clone(), taxi.escaped),
        );
        self.ground.release(airport, Self::owner(index));
        let heading = layout.stands()[stand.as_usize()].heading();
        let polyline = self.start_taxi(
            scenario,
            index,
            airport,
            Start {
                route,
                from: here,
                target: Target::Stand(stand),
                base,
                offset,
                extension: Vec::new(),
            },
            now,
        );
        // Taxiing on from a spot it reached regardless, it still is: it may
        // stand where another does.
        if let Some(taxi) = self.taxis[index].as_mut() {
            taxi.escaped = escaped;
        }
        if let (Some(path), Some(polyline)) = (self.paths[index].as_mut(), polyline) {
            let length = path.set_taxi_in(polyline, heading);
            self.profiles[index].set_taxi_in_length(length);
        }
        false
    }
}

/// The exit `planned` and those after it on the same runway end, nearest
/// first.
fn later_exits(layout: &AirportGround, planned: Id<Node>) -> Vec<Id<Node>> {
    let Some(first) = layout.exits().iter().find(|e| e.node() == planned) else {
        return vec![planned];
    };
    let mut later: Vec<&Exit> = layout
        .exits()
        .iter()
        .filter(|e| e.runway() == first.runway() && e.distance() >= first.distance())
        .collect();
    later.sort_by(|a, b| a.distance().value().total_cmp(&b.distance().value()));
    let mut nodes: Vec<Id<Node>> = Vec::with_capacity(later.len() + 1);
    nodes.push(planned);
    nodes.extend(later.iter().map(|e| e.node()).filter(|&n| n != planned));
    nodes.dedup();
    nodes
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed,
        ops::{StartMode, fleet::Operations},
        schedule::{ScheduleConfig, generate},
        test_data::{data, ground},
        units::Meters,
    };

    #[test]
    fn test_waiting_spots_lead_on_to_a_stand() -> Result<()> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?
                .with_ground(ground()?)?;
        let start: SimTime = "2026-07-08T04:00:00Z".parse()?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(3), start))?;
        let mut ops =
            Operations::new(&scenario, schedule, start, RngSeed::new(3), StartMode::Cold)?;
        ops.wingspans[0] = Meters::new(35.8)?;
        // Örebro has a spot from which no stand can be reached.
        let mut checked = 0;
        for ident in ["ESOE", "EDDF", "EDDM"] {
            let airport = scenario
                .world()
                .airport_by_ident(ident.parse()?)
                .context("airport")?;
            let layout = scenario.ground().airport(airport).context("layout")?;
            for exit in layout.exits() {
                let Some(spot) = ops.free_spot(&scenario, 0, airport, exit.node()) else {
                    continue;
                };
                let reach = layout.reachable_from(spot, ops.wingspans[0]);
                let stand = layout
                    .stands()
                    .iter()
                    .any(|s| s.max_wingspan() >= ops.wingspans[0] && reach[s.node().as_usize()]);
                assert!(stand, "{ident}: spot {spot:?} from exit {:?}", exit.node());
                checked += 1;
            }
        }
        assert!(checked > 20, "{checked}");
        Ok(())
    }
}
