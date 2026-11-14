//! Where each tail is when a run starts (D44).

use anyhow::{Context, Result};

use super::{
    BOARDING, Operations, TailOps, add,
    remote::{Parking, starts_remote},
    routed_taxi_out, seconds_between,
};
use crate::{
    SimDuration, SimTime,
    ops::{
        Phase, StartMode,
        motion::Motion,
        tail::{draw_delay, next_leg, off_block},
        timeline::{place, week_of},
    },
    scenario::Scenario,
    schedule::ScheduledFlight,
    time::{LocalDate, Zone},
};

/// Warm starts fast-forward legs in steps of this many seconds.
const WARM_STEP: f64 = 10.0;
/// Nominal taxi times for warm starts, s: out and in, normal and major.
const NOMINAL_TAXI: [[f64; 2]; 2] = [[540.0, 300.0], [870.0, 510.0]];

/// The fixed per-tail facts both start modes need.
pub(super) struct Rotation<'a> {
    pub(super) flights: &'a [ScheduledFlight],
    pub(super) zone: &'a Zone,
    pub(super) turnaround: SimDuration,
}

impl Rotation<'_> {
    pub(super) fn scheduled(&self, leg: usize, week: LocalDate) -> Result<SimTime> {
        let flight = self.flights.get(leg).context("leg out of range")?;
        place(self.zone, week, flight.departure())
    }

    /// The last leg instance scheduled to leave at or before `time`.
    fn current(&self, time: SimTime) -> Result<(usize, LocalDate, SimTime)> {
        let mut week = week_of(self.zone, time)?.add_days(-7)?;
        let mut leg = 0;
        let mut scheduled = self.scheduled(leg, week)?;
        loop {
            let (next, next_week) = next_leg(leg, self.flights.len(), week)?;
            let next_scheduled = self.scheduled(next, next_week)?;
            if next_scheduled > time {
                return Ok((leg, week, scheduled));
            }
            (leg, week, scheduled) = (next, next_week, next_scheduled);
        }
    }
}

impl Operations {
    /// Parks tail `index` before leg `leg`, which leaves at `off_block`:
    /// at a mapped airport on a free stand, or remote if it leaves late or
    /// none is free (D125); elsewhere as before.
    fn park_at_start(
        &mut self,
        scenario: &Scenario,
        index: usize,
        leg: usize,
        off_block: SimTime,
    ) -> Result<()> {
        let origin = self.flight(index, leg)?.origin();
        if scenario.ground().airport(origin).is_none() {
            return self.stand_for_leg(scenario, index, leg);
        }
        let near = scenario
            .world()
            .airport(origin)
            .context("origin out of range")?
            .location();
        let airline = self.airline_of(index, leg)?;
        if starts_remote(self.time, off_block)
            || self
                .find_stand(scenario, index, origin, airline, (near, None), false)
                .is_none()
        {
            self.parking[index] = Parking::Remote(None);
        }
        Ok(())
    }

    /// The tail's state at the start instant (D44).
    pub(super) fn place_tail(
        &mut self,
        scenario: &Scenario,
        index: usize,
        mode: StartMode,
    ) -> Result<TailOps> {
        let start = self.time;
        let rotation = self.rotation(index)?;
        let count = rotation.flights.len();
        let turnaround = rotation.turnaround;
        let (leg, week, scheduled) = rotation.current(start)?;
        let (next, next_week) = next_leg(leg, count, week)?;
        let next_scheduled = rotation.scheduled(next, next_week)?;
        if mode == StartMode::Cold {
            let delay = draw_delay(&mut self.rng)?;
            let leaves = off_block(next_scheduled, delay, start)?;
            self.park_at_start(scenario, index, next, leaves)?;
            return Ok(TailOps {
                leg: next,
                week: next_week,
                scheduled: next_scheduled,
                off_block: leaves,
                boarding_from: leaves.checked_sub(BOARDING)?.max(start),
                motion: None,
            });
        }
        // Warm: fly the current leg nominally from its scheduled departure,
        // from a stand that it has left by now: a free one, else from the
        // airport point; never shared, since it is not there.
        let origin = self.flight(index, leg)?.origin();
        let near = scenario
            .world()
            .airport(origin)
            .context("origin out of range")?
            .location();
        let airline = self.airline_of(index, leg)?;
        self.find_stand(scenario, index, origin, airline, (near, None), false);
        let planned = self.plan(scenario, index, leg)?;
        self.release_stand(index);
        let flight = *self.flight(index, leg)?;
        let taxi = [
            routed_taxi_out(&planned.path, &planned.profile)
                .unwrap_or(NOMINAL_TAXI[usize::from(self.is_major(flight.origin()))][0]),
            NOMINAL_TAXI[usize::from(self.is_major(flight.destination()))][1],
        ];
        let mut motion = Motion::start(&planned.profile, taxi[0], taxi[1]);
        let mut clock = scheduled;
        while clock < start && motion.phase() != Phase::Arrived {
            let dt = seconds_between(clock, start).min(WARM_STEP);
            motion.advance_free(&planned.profile, dt);
            clock = add(clock, SimDuration::from_millis(dt_millis(dt)))?;
        }
        if motion.phase() != Phase::Arrived {
            self.install(index, planned);
            if matches!(motion.phase(), Phase::Landing | Phase::TaxiIn) {
                let airline = self.airline_of(index, leg)?;
                if let Some(seconds) =
                    self.stand_on_arrival(scenario, index, flight.destination(), airline)
                {
                    motion.set_taxi_in(seconds);
                }
            }
            return Ok(TailOps {
                leg,
                week,
                scheduled,
                off_block: scheduled,
                boarding_from: scheduled.checked_sub(BOARDING)?,
                motion: Some(motion),
            });
        }
        let leaves = off_block(next_scheduled, SimDuration::ZERO, add(clock, turnaround)?)?;
        let boarding_from = leaves.checked_sub(BOARDING)?.max(clock);
        self.park_at_start(scenario, index, next, leaves)?;
        let mut tail = TailOps {
            leg: next,
            week: next_week,
            scheduled: next_scheduled,
            off_block: leaves,
            boarding_from,
            motion: None,
        };
        // Boarding already, if it has a stand; from remote parking it is
        // towed in first.
        if boarding_from <= start && self.parking[index] == Parking::Stand {
            let planned = self.plan(scenario, index, next)?;
            tail.motion = Some(Motion::boarding(&planned.profile));
            self.install(index, planned);
        }
        Ok(tail)
    }
}

/// Whole milliseconds of a step of `dt` seconds.
#[allow(clippy::cast_possible_truncation)]
fn dt_millis(dt: f64) -> i64 {
    // SAFETY: warm-start steps are at most `WARM_STEP` seconds.
    (dt * 1000.0).round() as i64
}
