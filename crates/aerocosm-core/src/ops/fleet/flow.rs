//! Runway control in the fleet: booking landing slots, absorbing delay and
//! answering clearance requests (plan M3 D56, D57).

use anyhow::{Context, Result};

use super::Operations;
use crate::{
    SimDuration, SimTime, WakeCategory,
    ops::{
        Phase,
        control::{RunwayKey, level_altitude, plan_arrival},
        motion::{Event, FINAL_LENGTH, LINEUP_SPEED, Motion, Profile, Request},
    },
};

/// Departures expecting more airborne delay than this wait at the gate.
const GROUND_DELAY_THRESHOLD: SimDuration = SimDuration::from_minutes(10);
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
    /// Landing slot booked at off-block (D58), reused when sequencing.
    provisional: Option<SimTime>,
    /// The off-block check has run for this leg.
    regulated: bool,
    /// Calculated take-off time while held at the gate.
    pub(super) slot_time: Option<SimTime>,
    /// Booked landing slot once sequenced.
    pub(super) landing: Option<SimTime>,
}

impl Flow {
    /// End of the planned hold.
    pub(super) const fn hold_until(&self) -> Option<SimTime> {
        self.hold_until
    }
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
        // Keep the slot booked at off-block if the flight can still make it.
        let slot = match flow.provisional {
            Some(slot) if slot >= eta => slot,
            stale => {
                if let Some(slot) = stale {
                    self.control.cancel_arrival(key, slot);
                }
                self.control
                    .book_arrival(key, eta, wake, profile.approach_speed())
            }
        };
        self.flows[index].landing = Some(slot);
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

    /// At off-block: books a provisional landing slot at a controlled
    /// destination and, if it lies more than [`GROUND_DELAY_THRESHOLD`]
    /// after the estimate, returns the ground delay to wait at the gate
    /// instead of in the air (D58).
    pub(super) fn regulate(
        &mut self,
        index: usize,
        leg: usize,
        motion: Motion,
        now: SimTime,
    ) -> Result<Option<SimDuration>> {
        let flow = self.resolve(index, leg)?;
        if flow.regulated {
            return Ok(None);
        }
        self.flows[index].regulated = true;
        let Some(key) = flow.arrival else {
            return Ok(None);
        };
        let profile = self.profiles[index];
        let Some(prediction) = predict(motion, &profile, profile.enroute_length()) else {
            return Ok(None);
        };
        let eta = after_seconds(now, prediction.touchdown);
        let slot = self
            .control
            .book_arrival(key, eta, self.wakes[index], profile.approach_speed());
        self.flows[index].provisional = Some(slot);
        // Whole seconds, so the gate time falls on a tick.
        let delay =
            SimDuration::from_millis((slot.duration_since(eta).as_millis() + 999) / 1000 * 1000);
        if delay <= GROUND_DELAY_THRESHOLD {
            return Ok(None);
        }
        self.stats.regulated(key.airport, delay);
        self.flows[index].slot_time = now.checked_add(delay).ok();
        Ok(Some(delay))
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
        let lineup = after_seconds(now, profile.lineup_length() / LINEUP_SPEED).duration_since(now);
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
                let clear = control.may_line_up(key, wake, now, lineup);
                if clear {
                    control.took_off(key, wake, now, lineup);
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
