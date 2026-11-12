//! Departures under ground control: the pushback clearance and departure
//! management at the holding positions (plan M4 D78, D82).

use anyhow::{Context, Result};

use super::{
    DEPARTURE_INTERVAL, ENGINE_START, GATE_ESCAPE_AFTER, GATE_RETRY, MAX_INTERVAL, MAX_TAXI_WAIT,
    PUSH_CLEAR, PUSH_SPEED, Start, Target, ms, taxi_millis,
};
use crate::{
    Id, SimDuration, SimTime,
    ground::Node,
    ops::{fleet::Operations, path::Path},
    scenario::Scenario,
    world::Airport,
};

impl Operations {
    /// At off-block of a routed departure from a controlled airport: clears
    /// the pushback and starts the taxi to the holding position, unless the
    /// departure would queue for long (departure management) or its lead-in
    /// and the taxilane behind it are taken: then it waits at the stand and
    /// asks again after the returned time.
    pub(in crate::ops::fleet) fn plan_departure(
        &mut self,
        scenario: &Scenario,
        index: usize,
        origin: Id<Airport>,
        now: SimTime,
    ) -> Result<Option<SimDuration>> {
        let Some((hold, _)) = self.paths[index].as_ref().and_then(Path::departure_nodes) else {
            return Ok(None);
        };
        let Some(layout) = scenario.ground().airport(origin) else {
            return Ok(None);
        };
        let parked = self.parked[index]
            .filter(|(at, _)| *at == origin)
            .and_then(|(_, stand)| layout.stands().get(stand.as_usize()));
        let stand = parked.map(crate::ground::Stand::node);
        let (Some(stand), true) = (stand, self.ground.controls(origin)) else {
            return Ok(None);
        };
        let Some(route) = self.route_for(layout, index, stand, hold) else {
            return Ok(None);
        };
        let push = self.profiles[index].pushback_length();
        let push_seconds = if push > 0.0 {
            push / PUSH_SPEED + ENGINE_START
        } else {
            0.0
        };
        // Departure management: no pushback while the runway queue is long.
        let turn = self
            .ground
            .airside(origin)
            .and_then(|a| a.next_departure.get(&hold).copied())
            .unwrap_or(i64::MIN);
        // SAFETY: a pushback takes minutes.
        #[allow(clippy::cast_possible_truncation)]
        let expected =
            ms(now) + (push_seconds * 1000.0) as i64 + taxi_millis(route.length().value() - push);
        let queue = turn.saturating_sub(expected);
        if queue > MAX_TAXI_WAIT {
            self.gate_blocked[index] = None;
            return Ok(Some(SimDuration::from_millis(queue - MAX_TAXI_WAIT)));
        }
        let since = *self.gate_blocked[index].get_or_insert(ms(now));
        let escape = ms(now) - since >= GATE_ESCAPE_AFTER;
        let queue = self.paths[index]
            .as_ref()
            .map_or_else(Vec::new, |p| p.queue_points().to_vec());
        let polyline = self
            .start_taxi(
                scenario,
                index,
                origin,
                Start {
                    route,
                    from: stand,
                    target: Target::Hold,
                    base: Vec::new(),
                    offset: 0.0,
                    extension: queue,
                },
                now,
            )
            .context("cannot build the taxi-out")?;
        // The pushback is approved with the taxilane clear where it ends and
        // nobody coming the other way (D82, D93).
        if let Some(taxi) = self.taxis[index].as_mut() {
            taxi.stand = parked.map(crate::ground::Stand::name);
            taxi.push = push;
            taxi.escaped = escape;
        }
        if !self.clear_way(index, push, push + PUSH_CLEAR, f64::INFINITY) {
            self.ground.release(origin, Self::owner(index));
            self.taxis[index] = None;
            return Ok(Some(SimDuration::from_millis(GATE_RETRY)));
        }
        if escape {
            self.stats.ground_conflict(origin);
        }
        self.gate_blocked[index] = None;
        if let Some(airside) = self.ground.airside_mut(origin) {
            let interval = airside.interval(hold);
            airside
                .next_departure
                .insert(hold, expected.max(turn) + interval);
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
        let Some(hold) = hold else {
            return;
        };
        // Someone else already waits for this holding position.
        let queued = self.taxis.iter().flatten().any(|taxi| {
            taxi.airport == airport && taxi.target == Target::Hold && taxi.end_node == Some(hold)
        });
        if let Some(airside) = self.ground.airside_mut(airport) {
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
}
