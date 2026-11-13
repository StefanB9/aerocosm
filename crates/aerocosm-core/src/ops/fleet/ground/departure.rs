//! Departures under ground control: the pushback clearance and departure
//! management at the holding positions (plan M4 D78, D82).

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use super::{
    DEPARTURE_INTERVAL, ENGINE_START, GATE_ESCAPE_AFTER, GATE_RETRY, MAX_INTERVAL, MAX_TAXI_WAIT,
    PUSH_CLEAR, PUSH_SPEED, Start, Target, ms, taxi_millis,
};
use crate::{
    Id, SimDuration, SimTime,
    ground::{AirportGround, Node},
    ops::{fleet::Operations, path::Path},
    scenario::Scenario,
    world::{Airport, RunwayDesignator},
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
        let runway = self
            .ground
            .airside(origin)
            .and_then(|a| a.hold_runway.get(&hold).copied());
        // Its turn on the runway, behind the departures cleared before it
        // (plan M4 D91).
        let turn = self
            .ground
            .airside(origin)
            .zip(runway)
            .map_or(i64::MIN, |(a, runway)| {
                let (last, pace) = a
                    .pace
                    .get(&runway)
                    .map_or((None, DEPARTURE_INTERVAL), |&(last, pace)| {
                        (Some(last), pace)
                    });
                let ahead = a.departing.get(&runway).copied().unwrap_or(0);
                self::turn(last, pace, ahead, ms(now))
            });
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
        if let (Some(airside), Some(runway)) = (self.ground.airside_mut(origin), runway) {
            *airside.departing.entry(runway).or_insert(0) += 1;
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
        let Some(runway) = self
            .ground
            .airside(airport)
            .and_then(|a| a.hold_runway.get(&hold).copied())
        else {
            return;
        };
        // Someone else already waits at this runway, at any of its holding
        // positions (plan M4 D91): only then does the interval show what the
        // runway can take.
        let queued = self.taxis.iter().enumerate().any(|(k, taxi)| {
            taxi.as_ref().is_some_and(|taxi| {
                taxi.airport == airport
                    && taxi.target == Target::Hold
                    && self
                        .along_of(k)
                        .is_some_and(|along| along >= taxi.end - QUEUE_REACH)
                    && self
                        .ground
                        .airside(airport)
                        .and_then(|a| taxi.end_node.and_then(|n| a.hold_runway.get(&n)))
                        == Some(&runway)
            })
        });
        if let Some(airside) = self.ground.airside_mut(airport) {
            let (last, mut pace) = airside
                .pace
                .get(&runway)
                .copied()
                .unwrap_or((i64::MIN, DEPARTURE_INTERVAL));
            // Only once a departure has gone before is there an interval.
            pace = learn_pace(pace, (queued && last > i64::MIN).then(|| ms(now) - last));
            airside.pace.insert(runway, (ms(now), pace));
        }
    }
}

/// A departure this near its holding position waits in the runway's queue,
/// m: first or second in line.
const QUEUE_REACH: f64 = 120.0;

/// The interval between departures learnt from `pace` and the interval
/// `seen` while a queue waited, ms; without a queue the runway was not
/// the limit, and the estimate relaxes towards the nominal interval.
fn learn_pace(pace: i64, seen: Option<i64>) -> i64 {
    let towards = seen.map_or(DEPARTURE_INTERVAL, |seen| {
        seen.clamp(DEPARTURE_INTERVAL, MAX_INTERVAL)
    });
    (pace * 7 + towards * 3) / 10
}

/// When the next departure cleared now may take off, ms: one interval
/// `pace` after the last line-up `last` (or now, if that is later) and one
/// more for each of the `ahead` departures cleared before it.
fn turn(last: Option<i64>, pace: i64, ahead: usize, now: i64) -> i64 {
    let first = last.map_or(now, |last| (last + pace).max(now));
    first + pace * i64::try_from(ahead).unwrap_or(i64::MAX / pace.max(1))
}

/// The departure runway each holding position of `layout` feeds.
pub(super) fn hold_runways(layout: &AirportGround) -> BTreeMap<Id<Node>, RunwayDesignator> {
    layout
        .holding_positions()
        .iter()
        .map(|h| (h.node(), h.runway()))
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{ground::GroundNetwork, test_data::data};

    #[test]
    fn test_a_departure_takes_its_turn_behind_those_cleared_before() {
        // Last line-up at 100 s, one every 90 s.
        assert_eq!(turn(Some(100_000), 90_000, 0, 120_000), 190_000);
        // Two cleared before it: two intervals later.
        assert_eq!(turn(Some(100_000), 90_000, 2, 120_000), 370_000);
        // Long quiet: its turn comes at once.
        assert_eq!(turn(Some(100_000), 90_000, 0, 900_000), 900_000);
        assert_eq!(turn(None, 90_000, 1, 900_000), 990_000);
    }

    #[test]
    fn test_the_interval_is_learnt_from_queues_and_relaxes_without() {
        // A queue moved every 150 s: the estimate rises towards it.
        assert_eq!(learn_pace(60_000, Some(150_000)), 87_000);
        // Never above the longest interval.
        assert_eq!(learn_pace(60_000, Some(900_000)), 132_000);
        // Nobody waited: back towards the nominal interval.
        assert_eq!(learn_pace(200_000, None), 158_000);
    }

    #[test]
    fn test_holding_positions_queue_for_their_runway() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let berlin = world.airport_by_ident("EDDB".parse()?).context("BER")?;
        let layout = network.airport(berlin).context("layout")?;
        let runways = hold_runways(layout);
        assert_eq!(runways.len(), layout.holding_positions().len());
        // Several holding positions feed each departure runway end.
        let on_24r = runways.values().filter(|r| r.to_string() == "24R").count();
        assert!(on_24r >= 2, "{on_24r}");
        Ok(())
    }
}
