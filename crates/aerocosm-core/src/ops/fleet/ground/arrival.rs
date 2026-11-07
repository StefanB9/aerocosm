//! Arrivals under ground control: stand or waiting spot at touchdown, and
//! the wait for a stand (plan M4 D69, D76).

use super::{EXIT_WAIT, Plan, RETRY, SPOT_ESCAPE_AFTER, SPOT_ESTIMATE, Start, Target, ms};
use crate::{
    Id, SimTime,
    ground::Node,
    ops::fleet::Operations,
    scenario::Scenario,
    world::{Airport, codes::AirlineIcao},
};

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
        // SAFETY: a landing roll takes minutes.
        #[allow(clippy::cast_possible_truncation)]
        let start = ms(now) + (self.profiles[index].landing_seconds() * 1000.0) as i64;
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
        let (to, target, stay, open) = match (stand, spot) {
            (Some(stand), _) => (
                layout.stands()[stand.as_usize()].node(),
                Target::Stand(stand),
                0,
                false,
            ),
            (None, Some(spot)) => (spot, Target::Spot(spot), SPOT_ESTIMATE, true),
            (None, None) => return false,
        };
        let plan = Plan {
            start_wait: EXIT_WAIT,
            shared: false,
            airport: destination,
            from: exit,
            to,
            start,
            stay,
            open,
            first: false,
        };
        // Never wait long on the runway: escape if nothing fits soon.
        let (route, escaped) = match self.plan_taxi(scenario, index, &plan, false) {
            Some(route) => (Some(route), false),
            None => (self.plan_taxi(scenario, index, &plan, true), true),
        };
        let Some(route) = route else {
            return false;
        };
        if escaped {
            self.stats.ground_conflict(destination);
        }
        if let Target::Spot(node) = target {
            self.stats.waited_for_stand(destination);
            if let Some(airside) = self.ground.airside_mut(destination)
                && let Some(slot) = airside.spots.iter().position(|s| *s == node)
            {
                airside.spot_owner[slot] = Some(Self::owner(index));
            }
        }
        let heading = stand.map(|s| layout.stands()[s.as_usize()].heading());
        let Some(polyline) = self.start_taxi(
            scenario,
            index,
            &plan,
            route,
            Start {
                target,
                base: Vec::new(),
                offset: 0.0,
                extension: Vec::new(),
            },
        ) else {
            return false;
        };
        if escaped {
            self.mark_escaped(index);
        }
        if let Some(path) = self.paths[index].as_mut() {
            let length = path.set_taxi_in(polyline, heading.unwrap_or_default());
            self.profiles[index].set_taxi_in_length(length);
        }
        true
    }

    /// The free waiting spot with the shortest taxi from `from` for tail
    /// `index`.
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
        airside
            .spots
            .iter()
            .zip(&airside.spot_owner)
            .filter(|&(&spot, owner)| owner.is_none() && length(spot).is_finite())
            .map(|(spot, _)| *spot)
            .min_by(|a, b| length(*a).total_cmp(&length(*b)))
    }

    /// At a waiting spot: takes a stand as soon as one is free and taxies
    /// there.
    pub(super) fn wait_for_stand(
        &mut self,
        scenario: &Scenario,
        index: usize,
        now: SimTime,
        airline: AirlineIcao,
    ) {
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        if taxi.waiting.is_some_and(|retry| ms(now) < retry) {
            return;
        }
        let (airport, at) = (taxi.airport, taxi.route.steps.len().saturating_sub(1));
        let escape = ms(now) - taxi.spot_since >= SPOT_ESCAPE_AFTER;
        let Some(here) = taxi.route.steps.last().map(|s| s.node) else {
            return;
        };
        let Some(near) = scenario
            .ground()
            .airport(airport)
            .map(|l| l.nodes()[here.as_usize()].position())
        else {
            return;
        };
        // After half an hour it shares a stand rather than wait on (D69).
        let free = self.find_stand(scenario, index, airport, airline, (near, Some(here)), false);
        let shared = free.is_none() && escape;
        let stand = free.or_else(|| {
            shared
                .then(|| self.take_stand(scenario, index, airport, airline, (near, Some(here))))
                .flatten()
        });
        let Some(stand) = stand else {
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.waiting = Some(ms(now) + RETRY);
            }
            return;
        };
        if shared {
            self.stats.shared_stand(airport);
        }
        let Some(layout) = scenario.ground().airport(airport) else {
            return;
        };
        let plan = Plan {
            start_wait: i64::MAX,
            shared: true,
            airport,
            from: here,
            to: layout.stands()[stand.as_usize()].node(),
            start: ms(now),
            stay: 0,
            open: false,
            first: false,
        };
        let owner = Self::owner(index);
        // Keep the stand and try the route again shortly; after half an hour
        // go regardless (D75).
        let (route, forced) = match self.plan_taxi(scenario, index, &plan, false) {
            Some(route) => (Some(route), false),
            None if escape => (self.plan_taxi(scenario, index, &plan, true), true),
            None => (None, false),
        };
        let Some(route) = route else {
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.waiting = Some(ms(now) + RETRY);
            }
            return;
        };
        self.ground.release(airport, owner);
        let heading = layout.stands()[stand.as_usize()].heading();
        if let Some(taxi) = self.taxis[index].as_mut() {
            taxi.target = Target::Stand(stand);
            taxi.waiting = None;
        }
        if let Some(path) = self.paths[index].as_mut() {
            path.set_stand_in(heading);
        }
        self.continue_from(scenario, index, &plan, route, at);
        if forced {
            self.stats.ground_conflict(airport);
            self.mark_escaped(index);
        }
    }
}
