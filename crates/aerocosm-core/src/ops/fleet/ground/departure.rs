//! Departures under ground control: pushback and taxi plans, and departure
//! management at the holding positions (plan M4 D70, D78).

use anyhow::{Context, Result};

use super::{
    DEPARTURE_INTERVAL, ENGINE_START, GATE_ESCAPE_AFTER, GATE_RETRY, MAX_INTERVAL, MAX_TAXI_WAIT,
    PUSH_SPEED, Plan, QUEUE_ESTIMATE, RETRY, Start, Target, backoff, ms, waits,
};
use crate::{
    Id, SimDuration, SimTime,
    ground::{
        Node, TAXI_SPEED,
        reserve::{NODE_BUFFER, TimedRoute},
    },
    ops::{fleet::Operations, path::Path},
    scenario::Scenario,
    world::Airport,
};

impl Operations {
    /// At off-block of a routed departure from a controlled airport: plans
    /// the pushback and taxi to the holding position, unless the departure
    /// would queue for long: then it waits at the stand for its turn
    /// (departure management) and asks again after the returned time.
    pub(in crate::ops::fleet) fn plan_departure(
        &mut self,
        scenario: &Scenario,
        index: usize,
        origin: Id<Airport>,
        now: SimTime,
    ) -> Result<Option<SimDuration>> {
        let Some((hold, joint)) = self.paths[index].as_ref().and_then(Path::departure_nodes) else {
            return Ok(None);
        };
        if !self.ground.controls(origin) {
            return Ok(None);
        }
        let push = self.profiles[index].pushback_length();
        let push_seconds = if push > 0.0 {
            push / PUSH_SPEED + ENGINE_START
        } else {
            0.0
        };
        // SAFETY: a pushback takes minutes.
        #[allow(clippy::cast_possible_truncation)]
        let start = ms(now) + (push_seconds * 1000.0) as i64;
        let plan = Plan {
            start_wait: i64::MAX,
            shared: false,
            airport: origin,
            from: joint,
            to: hold,
            start,
            stay: QUEUE_ESTIMATE,
            open: true,
        };
        // Departure management first, from the unimpeded taxi along the
        // planned route: no planning while the runway queue is long.
        let turn = self
            .ground
            .airside(origin)
            .and_then(|a| a.next_departure.get(&hold).copied())
            .unwrap_or(i64::MIN);
        let profile = &self.profiles[index];
        // SAFETY: taxi times are minutes.
        #[allow(clippy::cast_possible_truncation)]
        let expected =
            start + ((profile.taxi_out_length() - push).max(0.0) / TAXI_SPEED * 1000.0) as i64;
        let queue = turn.saturating_sub(expected);
        if queue > MAX_TAXI_WAIT {
            self.gate_blocked[index] = None;
            return Ok(Some(SimDuration::from_millis(queue - MAX_TAXI_WAIT)));
        }
        let since = *self.gate_blocked[index].get_or_insert(ms(now));
        let escape = ms(now) - since >= GATE_ESCAPE_AFTER;
        let owner = Self::owner(index);
        let joint_free = self.ground.airside(origin).is_some_and(|a| {
            !a.table
                .node_taken(joint, ms(now), start + NODE_BUFFER, owner)
        });
        let route = if joint_free || escape {
            self.plan_taxi(scenario, index, &plan, escape)
        } else {
            None
        };
        let retry = SimDuration::from_millis(backoff(ms(now) - since, GATE_RETRY));
        let Some(route) = route else {
            return Ok(Some(retry));
        };
        let arrival = route.arrival();
        if escape {
            self.stats.ground_conflict(origin);
        } else if let Some(wait) = self.too_early(index, &route, turn) {
            return Ok(Some(wait));
        }
        self.gate_blocked[index] = None;
        if let Some(airside) = self.ground.airside_mut(origin) {
            airside
                .table
                .occupy(joint, ms(now), start + NODE_BUFFER, owner);
            let interval = airside.interval(hold);
            airside
                .next_departure
                .insert(hold, arrival.max(turn) + interval);
        }
        let (base, queue) = self.paths[index]
            .as_ref()
            .map_or_else(Default::default, |p| {
                (p.push_points().to_vec(), p.queue_points().to_vec())
            });
        let polyline = self
            .start_taxi(
                scenario,
                index,
                &plan,
                route,
                Start {
                    target: Target::Hold,
                    base,
                    offset: push,
                    extension: queue,
                },
            )
            .context("cannot build the taxi-out")?;
        if escape {
            self.mark_escaped(index);
        }
        if let Some(path) = self.paths[index].as_mut() {
            let length = path.set_taxi_out(polyline);
            self.profiles[index].set_taxi_out_length(length);
        }
        Ok(None)
    }

    /// A departure lines up from `hold`: the next one there is due one
    /// interval later at the earliest, and the pace is learnt.
    pub(in crate::ops::fleet) fn lined_up(
        &mut self,
        airport: Id<Airport>,
        hold: Option<Id<Node>>,
        now: SimTime,
    ) {
        if let (Some(airside), Some(hold)) = (self.ground.airside_mut(airport), hold) {
            let queued = airside.table.windows(hold).iter().any(|w| w.open);
            let (last, mut pace) = airside
                .pace
                .get(&hold)
                .copied()
                .unwrap_or((i64::MIN, DEPARTURE_INTERVAL));
            if queued && last > i64::MIN {
                let seen = (ms(now) - last).clamp(DEPARTURE_INTERVAL, MAX_INTERVAL);
                pace = (pace * 7 + seen * 3) / 10;
            }
            airside.pace.insert(hold, (ms(now), pace));
            let next = airside.next_departure.entry(hold).or_insert(i64::MIN);
            *next = (*next).max(ms(now) + pace);
        }
    }

    /// Why a departure with `route` should not leave yet: its turn at the
    /// runway is too far off, or the route waits too long on the way.
    fn too_early(&mut self, index: usize, route: &TimedRoute, turn: i64) -> Option<SimDuration> {
        let unimpeded = route.arrival() - waits(route);
        let queue = turn.saturating_sub(unimpeded);
        if queue > MAX_TAXI_WAIT {
            self.gate_blocked[index] = None;
            return Some(SimDuration::from_millis((queue - MAX_TAXI_WAIT).max(RETRY)));
        }
        (waits(route) > MAX_TAXI_WAIT).then_some(SimDuration::from_millis(GATE_RETRY))
    }
}
