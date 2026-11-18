//! Which departure runway a flight takes where the mode in use offers
//! several (plan M6 D151): about to push back, it sees each runway's queue,
//! which leg planning does not know.

use anyhow::Result;

use super::Operations;
use crate::{SimTime, ops::control::RunwayKey, scenario::Scenario};

/// A flight changes runway only for this much less time, s.
const WORTH_CHANGING: f64 = 60.0;
/// A flight held at its gate looks at its runways again this often, ms.
const LOOK_AGAIN: i64 = 300_000;

/// A wait as a flight about to push back weighs it, s: the runway it
/// planned on counts for a little less, so that it changes only for a real
/// gain.
pub(super) fn stay(wait: f64, planned: bool) -> f64 {
    if planned {
        (wait - WORTH_CHANGING).max(0.0)
    } else {
        wait
    }
}

/// Taxi-out and climb before the en-route flight, roughly, for the time a
/// flight about to push back lands, s.
const BEFORE_ENROUTE: f64 = 600.0;
/// Metres per second in a knot.
const KNOT: f64 = 0.514_444;

impl Operations {
    /// About to push back at `now`, how long after it would land tail
    /// `index` waits for a landing slot on the runway of `end` at
    /// `destination`, s (plan M7 D159).
    pub(super) fn land_wait(
        &self,
        scenario: &Scenario,
        index: usize,
        destination: crate::Id<crate::world::Airport>,
        end: &crate::world::RunwayEnd,
        now: SimTime,
    ) -> f64 {
        if !self.control.is_sequenced(destination) {
            return 0.0;
        }
        let Some(runway) = scenario
            .world()
            .runways_of(destination)
            .iter()
            .position(|r| r.ends().iter().any(|e| e.designator() == end.designator()))
        else {
            return 0.0;
        };
        let profile = &self.profiles[index];
        let cruise = scenario
            .traffic()
            .and_then(|t| {
                let leg = self.tails[index].leg;
                let flight = self.flight(index, leg).ok()?;
                t.aircraft().get(flight.aircraft())
            })
            .map_or(230.0, |aircraft| aircraft.cruise_speed().value() * KNOT);
        let seconds = BEFORE_ENROUTE + profile.enroute_length() / cruise.max(1.0);
        // SAFETY: hours of flight, far inside i64 milliseconds.
        #[allow(clippy::cast_possible_truncation)]
        let Ok(eta) = now.checked_add(crate::SimDuration::from_millis((seconds * 1000.0) as i64))
        else {
            return 0.0;
        };
        let key = RunwayKey {
            airport: destination,
            runway,
        };
        self.control
            .landing_wait(key, eta, self.wakes[index], profile.approach_speed())
            .as_secs_f64()
    }

    /// About to push back at `now`, tail `index` takes another departure
    /// runway if it would be off sooner from it, the queues as they are
    /// (plan M6 D151); the rest of its leg stays.
    pub(super) fn rechoose_departure(
        &mut self,
        scenario: &Scenario,
        index: usize,
        now: SimTime,
    ) -> Result<()> {
        if now.as_millis().saturating_sub(self.rechosen[index]) < LOOK_AGAIN {
            return Ok(());
        }
        self.rechosen[index] = now.as_millis();
        let leg = self.tails[index].leg;
        let origin = self.flight(index, leg)?.origin();
        let planned = self.paths[index]
            .as_ref()
            .and_then(crate::ops::path::Path::departure_end);
        if planned.is_none() || !self.ground.controls(origin) {
            return Ok(());
        }
        let again = self.plan_at(scenario, index, leg, Some(now))?;
        let lands = self.paths[index].as_ref().and_then(|path| path.runways().1);
        let lands_again = again.path.runways().1;
        if again.path.departure_end() == planned && lands_again == lands {
            return Ok(());
        }
        // A landing runway changed: the provisional slot on the old one is
        // freed and the new one booked when the flight is sequenced.
        let destination = self.flight(index, leg)?.destination();
        let arrival = lands_again
            .filter(|_| self.control.is_sequenced(destination))
            .map(|runway| RunwayKey {
                airport: destination,
                runway,
            });
        if lands_again != lands
            && let Some(flow) = self.flows.get_mut(index)
            && flow.is_resolved()
            && let Some((key, slot)) = flow.set_arrival(arrival)
        {
            self.control.cancel_arrival(key, slot);
        }
        // Its runway control follows; what it has booked at the destination
        // stays.
        let departure = again
            .path
            .runways()
            .0
            .filter(|_| self.control.is_sequenced(origin))
            .map(|runway| RunwayKey {
                airport: origin,
                runway,
            });
        if let Some(flow) = self.flows.get_mut(index)
            && flow.is_resolved()
        {
            flow.set_departure(departure);
        }
        self.profiles[index] = again.profile;
        self.paths[index] = Some(again.path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context as _;

    use super::*;
    use crate::{
        RngSeed,
        ops::{Phase, StartMode, path::Path},
        schedule::{ScheduleConfig, generate},
        test_data::data,
    };

    #[test]
    fn test_a_planned_runway_is_left_only_for_a_real_gain() {
        assert!((stay(300.0, false) - 300.0).abs() < 1e-9);
        assert!((stay(300.0, true) - 240.0).abs() < 1e-9);
        assert!(stay(20.0, true).abs() < 1e-9);
    }

    fn mapped(seed: u64) -> Result<(Scenario, Operations)> {
        let start: SimTime = "2026-07-08T10:00:00Z".parse()?;
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?
                .with_ground(crate::test_data::ground()?)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), start))?;
        let operations = Operations::new(
            &scenario,
            schedule,
            start,
            RngSeed::new(seed),
            StartMode::Warm,
        )?;
        Ok((scenario, operations))
    }

    #[test]
    fn test_a_departure_leaves_a_runway_whose_queue_is_long() -> Result<()> {
        let (scenario, mut operations) = mapped(2)?;
        let munich = scenario
            .world()
            .airport_by_ident("EDDM".parse()?)
            .context("EDDM")?;
        let mut changed = 0;
        for _ in 0..40 {
            for _ in 0..30 {
                operations.step(&scenario)?;
            }
            let now = operations.time;
            for index in 0..operations.tails.len() {
                let boarding = operations.tails[index]
                    .motion
                    .is_some_and(|m| m.phase() == Phase::Boarding);
                let leg = operations.tails[index].leg;
                if !boarding || operations.flight(index, leg)?.origin() != munich {
                    continue;
                }
                let Some(path) = operations.paths[index].clone().filter(Path::routed_out) else {
                    continue;
                };
                let planned = path.departure_end().context("a runway end")?;
                let lands = path.runways().1;
                // Nothing to gain: it stays.
                operations.rechosen[index] = i64::MIN;
                operations.rechoose_departure(&scenario, index, now)?;
                let unchanged = operations.paths[index].as_ref().context("a path")?;
                assert_eq!(unchanged.departure_end(), Some(planned));
                // An hour between departures on its runway: the other one.
                operations
                    .ground
                    .set_pace(munich, planned, now.as_millis(), 3_600_000);
                // Not asked again within minutes…
                operations.rechoose_departure(&scenario, index, now)?;
                let still = operations.paths[index].as_ref().context("a path")?;
                assert_eq!(still.departure_end(), Some(planned));
                // …but then, landing where it was going to.
                operations.rechosen[index] = i64::MIN;
                operations.rechoose_departure(&scenario, index, now)?;
                let again = operations.paths[index].as_ref().context("a path")?;
                assert_ne!(again.departure_end(), Some(planned), "tail {index}");
                assert!(again.routed_out());
                assert_eq!(again.runways().1, lands);
                operations
                    .ground
                    .set_pace(munich, planned, now.as_millis(), 60_000);
                changed += 1;
            }
            if changed >= 3 {
                return Ok(());
            }
        }
        anyhow::bail!("{changed} departures from Munich checked")
    }
}
