//! Runway control in the fleet: booking landing slots, absorbing delay and
//! answering clearance requests (plan M3 D56, D57).

use anyhow::{Context, Result};

use super::Operations;
use crate::{
    SimDuration, SimTime, WakeCategory,
    ops::{
        Phase,
        control::{RunwayKey, level_altitude, plan_arrival},
        motion::{Event, FINAL_LENGTH, Motion, Profile, Request},
    },
};

/// Arrivals are sequenced from this far before the final fix, m.
const HORIZON: f64 = 200_000.0;
/// Holds are flown this far before the final fix along the route (35 NM).
const HOLD_DISTANCE: f64 = 64_820.0;
/// Shorter holds are not worth it: the gate at the fix takes the rest.
const MIN_HOLD_SECONDS: f64 = 30.0;
/// Descents ask for approach clearance within this distance of the fix, m:
/// more than a tick's flight at any descent speed.
const ASKING_DISTANCE: f64 = 500.0;
/// Landing time predictions step this many seconds.
const PREDICTION_STEP: f64 = 10.0;
/// Predictions give up after this many steps (a day).
const PREDICTION_LIMIT: usize = 8_640;

/// Runway control state of one tail's current leg.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Flow {
    /// Runway keys resolved for this leg.
    resolved: bool,
    departure: Option<RunwayKey>,
    arrival: Option<RunwayKey>,
    /// The landing slot has been booked (or the destination is free).
    booked: bool,
    /// End of the planned hold.
    hold_until: Option<SimTime>,
    /// Holding level taken over the destination.
    level: Option<usize>,
}

/// What a prediction found: touchdown and when the hold point is reached,
/// both in seconds from now.
struct Prediction {
    touchdown: f64,
    reach_hold: f64,
}

fn after_seconds(now: SimTime, seconds: f64) -> SimTime {
    // Flight times are hours at most, far inside i64 milliseconds.
    #[allow(clippy::cast_possible_truncation)]
    let millis = (seconds * 1000.0).round() as i64;
    now.checked_add(SimDuration::from_millis(millis))
        .unwrap_or(now)
}

/// Flies a copy of `motion` unhindered until touchdown.
fn predict(motion: Motion, profile: &Profile, hold_point: f64) -> Option<Prediction> {
    let mut copy = motion;
    copy.set_speed_factor(1.0);
    let mut elapsed = 0.0;
    let mut reach_hold = None;
    for _ in 0..PREDICTION_LIMIT {
        let event = copy.advance_free(profile, PREDICTION_STEP);
        elapsed += PREDICTION_STEP;
        if reach_hold.is_none()
            && matches!(copy.phase(), Phase::Climb | Phase::Cruise | Phase::Descent)
            && copy.along() >= hold_point
        {
            reach_hold = Some(elapsed);
        }
        if event == Some(Event::Landing) {
            return Some(Prediction {
                touchdown: elapsed,
                reach_hold: reach_hold.unwrap_or(0.0),
            });
        }
    }
    None
}

impl Operations {
    /// Resolves the runways of tail `index`'s leg if not done yet.
    fn resolve(&mut self, index: usize, leg: usize) -> Result<Flow> {
        let flow = self.flows[index];
        if flow.resolved {
            return Ok(flow);
        }
        let flight = *self.flight(index, leg)?;
        let (departure, arrival) = self.paths[index]
            .as_ref()
            .map_or((None, None), super::super::path::Path::runways);
        let key = |airport, runway: Option<usize>| {
            runway
                .filter(|_| self.control.is_sequenced(airport))
                .map(|runway| RunwayKey { airport, runway })
        };
        let resolved = Flow {
            resolved: true,
            departure: key(flight.origin(), departure),
            arrival: key(flight.destination(), arrival),
            ..Flow::default()
        };
        self.flows[index] = resolved;
        Ok(resolved)
    }

    /// Books a landing slot once the arrival is within the horizon, and
    /// plans speed control and holding for its delay.
    fn book(&mut self, index: usize, leg: usize, motion: &mut Motion, now: SimTime) -> Result<()> {
        let profile = self.profiles[index];
        let remaining = profile.enroute_length() - motion.along();
        let enroute = matches!(
            motion.phase(),
            Phase::Climb | Phase::Cruise | Phase::Descent
        );
        if self.flows[index].booked || !enroute || remaining > HORIZON {
            return Ok(());
        }
        let flow = self.resolve(index, leg)?;
        self.flows[index].booked = true;
        let Some(key) = flow.arrival else {
            return Ok(());
        };
        let hold_point = (profile.enroute_length() - HOLD_DISTANCE).max(motion.along());
        let Some(prediction) = predict(*motion, &profile, hold_point) else {
            return Ok(());
        };
        let wake = self.wakes[index];
        let eta = after_seconds(now, prediction.touchdown);
        let slot = self
            .control
            .book_arrival(key, eta, wake, profile.approach_speed());
        let delay = slot.duration_since(eta).as_secs_f64();
        let final_seconds = FINAL_LENGTH / profile.approach_speed();
        let plan = plan_arrival(delay, (prediction.touchdown - final_seconds).max(1.0));
        motion.set_speed_factor(plan.speed_factor);
        if plan.hold_seconds >= MIN_HOLD_SECONDS {
            let level = self.control.take_level(key.airport);
            motion.plan_hold(hold_point, level_altitude(level));
            let reach = prediction.reach_hold / plan.speed_factor;
            self.flows[index].hold_until = Some(after_seconds(now, reach + plan.hold_seconds));
            self.flows[index].level = Some(level);
        }
        Ok(())
    }

    /// Advances tail `index`'s motion by a tick, answering its clearance
    /// requests from runway control.
    pub(super) fn fly(
        &mut self,
        index: usize,
        leg: usize,
        motion: &mut Motion,
        now: SimTime,
        dt: f64,
    ) -> Result<Option<Event>> {
        // The common case first: most aircraft neither book nor ask for
        // anything this tick.
        let phase = motion.phase();
        let enroute = matches!(phase, Phase::Climb | Phase::Cruise | Phase::Descent);
        let remaining = self.profiles[index].enroute_length() - motion.along();
        if remaining <= HORIZON && enroute && !self.flows[index].booked {
            self.book(index, leg, motion, now)?;
        }
        let asking = matches!(phase, Phase::Queue | Phase::Holding)
            || (phase == Phase::Descent && remaining <= ASKING_DISTANCE);
        if !asking {
            return Ok(motion.advance_free(&self.profiles[index], dt));
        }
        let profile = self.profiles[index];
        let flow = self.resolve(index, leg)?;
        let wake: WakeCategory = self.wakes[index];
        let at_fix = motion.along() >= profile.enroute_length() - 1.0;
        let touchdown = after_seconds(now, FINAL_LENGTH / profile.approach_speed());
        let speed = profile.approach_speed();
        let control = &mut self.control;
        let hold_until = flow.hold_until;
        let held = motion.held_for() + dt;
        let mut released = false;
        let mut cleared_takeoff = false;
        let event = motion.advance(&profile, dt, &mut |request| match request {
            Request::Takeoff => flow.departure.is_none_or(|key| {
                let clear = control.may_take_off(key, wake, now);
                if clear {
                    control.took_off(key, wake, now);
                    cleared_takeoff = true;
                }
                clear
            }),
            Request::Approach => flow.arrival.is_none_or(|key| {
                let clear = control.may_approach(key, wake, speed, touchdown);
                if clear {
                    control.approach_started(key, wake, touchdown);
                }
                clear
            }),
            Request::LeaveHold => {
                let leave = if at_fix {
                    flow.arrival
                        .is_none_or(|key| control.may_approach(key, wake, speed, touchdown))
                } else {
                    hold_until.is_none_or(|until| now >= until)
                };
                released |= leave;
                leave
            }
        });
        if released {
            if let (Some(level), Some(key)) = (flow.level, flow.arrival) {
                self.control.release_level(key.airport, level);
            }
            if let Some(key) = flow.arrival {
                self.stats.held(key.airport, held);
            }
            self.flows[index].level = None;
            self.flows[index].hold_until = None;
        }
        if cleared_takeoff && let Some(key) = flow.departure {
            self.record(key, now, wake, None);
        }
        Ok(event)
    }

    /// Bookkeeping after a touchdown at a controlled runway.
    pub(super) fn touched_down(&mut self, index: usize, leg: usize, now: SimTime) -> Result<()> {
        let flow = self.resolve(index, leg)?;
        if let Some(key) = flow.arrival {
            self.control.landed(key, now);
            let speed = self.profiles[index].approach_speed();
            self.record(key, now, self.wakes[index], Some(speed));
        }
        Ok(())
    }

    /// Starts runway control afresh for a newly planned leg.
    pub(super) fn reset_flow(&mut self, index: usize) {
        if let Some(flow) = self.flows.get(index).copied()
            && let (Some(level), Some(key)) = (flow.level, flow.arrival)
        {
            self.control.release_level(key.airport, level);
        }
        if let Some(flow) = self.flows.get_mut(index) {
            *flow = Flow::default();
        }
    }

    /// Forgets old landing slots now and then.
    pub(super) fn prune_control(&mut self, now: SimTime) -> Result<()> {
        let cutoff = now
            .checked_sub(SimDuration::from_hours(1))
            .context("prune cutoff")?;
        self.control.prune(cutoff);
        Ok(())
    }

    /// Notes a take-off (`approach_speed` `None`) or touchdown for tests.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn record(
        &mut self,
        key: RunwayKey,
        now: SimTime,
        wake: WakeCategory,
        approach_speed: Option<f64>,
    ) {
        #[cfg(test)]
        self.log.push(super::RunwayUse {
            key,
            time: now,
            wake,
            approach_speed,
        });
        #[cfg(not(test))]
        let _ = (key, now, wake, approach_speed);
    }
}
