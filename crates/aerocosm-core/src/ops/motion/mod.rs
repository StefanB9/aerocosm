//! Per-leg kinematics on scalars (D36, D48): distance along the current
//! segment, altitude and speed, advanced without trigonometry.

mod airborne;
mod profile;

pub(crate) use self::profile::{Profile, ProfileInputs};
use super::Phase;

/// The hold point when no hold is planned.
const NO_HOLD: f64 = f64::INFINITY;

/// Narrows a value for compact storage; taxi times and speed factors need
/// no more than f32 precision.
#[allow(clippy::cast_possible_truncation)]
const fn compact(value: f64) -> f32 {
    value as f32
}

/// Take-off acceleration along the runway, m/s².
const TAKEOFF_ACCELERATION: f64 = 2.0;
/// Braking deceleration after touchdown, m/s².
const BRAKING: f64 = 1.8;
/// Speed at which an aircraft leaves the runway, m/s (about 15 kt).
const TAXI_SPEED: f64 = 8.0;
/// Pushback speed, m/s.
const PUSH_SPEED: f64 = 1.0;
/// Engine start after the pushback, s.
const ENGINE_START: f64 = 60.0;
/// Line-up speed from the holding position onto the runway, m/s.
pub(crate) const LINEUP_SPEED: f64 = 5.0;
/// Rotation speed relative to the approach speed.
const ROTATE_FACTOR: f64 = 1.15;
/// Initial climb speed relative to the approach speed.
const CLIMB_ENTRY_FACTOR: f64 = 1.3;
/// Speed at the final approach fix relative to the approach speed.
const DESCENT_EXIT_FACTOR: f64 = 1.4;
/// Final approach: 10 NM on a 3° glide path.
pub(crate) const FINAL_LENGTH: f64 = 18_520.0;
/// tan 3°.
const GLIDE_SLOPE: f64 = 0.052_407_779;

/// A clearance a moving aircraft asks for before it may continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// Line up and take off from the holding point.
    Takeoff,
    /// Pass the final approach fix and land.
    Approach,
    /// Leave the holding pattern.
    LeaveHold,
}

/// Something worth counting happened during a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    /// Lift-off: a departure movement.
    Takeoff,
    /// Touchdown: an arrival movement.
    Landing,
    /// On blocks at the arrival gate.
    Arrived,
}

/// Where an aircraft is on its leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Motion {
    phase: Phase,
    /// Distance along the current phase's segment, m.
    along: f64,
    altitude: f64,
    speed: f64,
    /// Time spent in a timed phase (taxi), s.
    elapsed: f64,
    /// Taxi times, s; f32 keeps the motion compact for the per-tick loop.
    taxi_out: f32,
    taxi_in: f32,
    /// Fraction of the normal speed flown in cruise and descent.
    speed_factor: f32,
    /// Route distance at which to enter a planned hold.
    /// [`NO_HOLD`] when none is planned.
    hold_at: f64,
    /// Level of the planned or current hold, m.
    hold_altitude: f64,
}

impl Motion {
    /// Pushes back at the origin gate with the given taxi times, s.
    pub(crate) fn start(profile: &Profile, taxi_out: f64, taxi_in: f64) -> Self {
        Self {
            phase: if profile.pushback_length() > 0.0 {
                Phase::Pushback
            } else {
                Phase::TaxiOut
            },
            along: 0.0,
            altitude: profile.inputs.origin_elevation,
            speed: 0.0,
            elapsed: 0.0,
            taxi_out: compact(taxi_out),
            taxi_in: compact(taxi_in),
            speed_factor: 1.0,
            hold_at: NO_HOLD,
            hold_altitude: 0.0,
        }
    }

    /// Boarding at the origin gate.
    pub(crate) const fn boarding(profile: &Profile) -> Self {
        Self {
            phase: Phase::Boarding,
            along: 0.0,
            altitude: profile.inputs.origin_elevation,
            speed: 0.0,
            elapsed: 0.0,
            taxi_out: 0.0,
            taxi_in: 0.0,
            speed_factor: 1.0,
            hold_at: NO_HOLD,
            hold_altitude: 0.0,
        }
    }

    /// Placed by ground control `along` the taxi route at `speed` m/s
    /// (plan M4 D70); the phase stays.
    pub(crate) const fn taxi_at(&mut self, along: f64, speed: f64) {
        self.along = along;
        self.speed = speed;
    }

    /// At the runway holding position: queues for take-off.
    pub(crate) const fn queue_now(&mut self) {
        self.enter(Phase::Queue);
        self.speed = 0.0;
    }

    /// On the stand: the leg is over.
    pub(crate) const fn arrive_now(&mut self) {
        self.enter(Phase::Arrived);
        self.speed = 0.0;
    }

    /// Taxies in for `seconds` after landing (known at touchdown).
    pub(crate) fn set_taxi_in(&mut self, seconds: f64) {
        self.taxi_in = compact(seconds);
    }

    /// Flies cruise and descent at `factor` of the normal speed (speed
    /// control by the arrival manager).
    pub(crate) fn set_speed_factor(&mut self, factor: f64) {
        self.speed_factor = compact(factor);
    }

    /// The current speed factor.
    pub(crate) fn speed_factor(&self) -> f64 {
        f64::from(self.speed_factor)
    }

    /// Holds at route distance `at` (m along the en-route part) at `altitude`
    /// (m) until released.
    pub(crate) fn plan_hold(&mut self, at: f64, altitude: f64) {
        self.hold_at = at;
        self.hold_altitude = altitude;
    }

    /// Whether a hold is planned and not yet left.
    #[cfg(test)]
    pub(crate) fn has_hold(&self) -> bool {
        self.hold_at < NO_HOLD
    }

    /// Seconds spent in the current hold.
    pub(crate) const fn held_for(&self) -> f64 {
        if matches!(self.phase, Phase::Holding) {
            self.elapsed
        } else {
            0.0
        }
    }

    /// Enters the hold where the aircraft is.
    fn enter_hold(&mut self, at: f64) {
        self.phase = Phase::Holding;
        self.along = at;
        self.elapsed = 0.0;
    }

    /// Whether the planned hold starts within the next `step` metres.
    fn hold_due(&self, step: f64) -> Option<f64> {
        (self.hold_at < NO_HOLD && self.along + step >= self.hold_at).then_some(self.hold_at)
    }

    /// Advances with every clearance granted, as when nothing controls the
    /// traffic.
    pub(crate) fn advance_free(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        self.advance(p, dt, &mut |_| true)
    }

    /// The current phase.
    pub(crate) const fn phase(&self) -> Phase {
        self.phase
    }

    /// Distance along the current phase's segment, m.
    pub(crate) const fn along(&self) -> f64 {
        self.along
    }

    /// Altitude above mean sea level, m.
    pub(crate) const fn altitude(&self) -> f64 {
        self.altitude
    }

    /// Ground speed, m/s.
    pub(crate) const fn speed(&self) -> f64 {
        self.speed
    }

    const fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.along = 0.0;
        self.elapsed = 0.0;
    }

    /// Advances by `dt` seconds; at most one phase change per call, any
    /// remainder of the step is dropped.
    pub(crate) fn advance(
        &mut self,
        p: &Profile,
        dt: f64,
        clearance: &mut impl FnMut(Request) -> bool,
    ) -> Option<Event> {
        let inputs = &p.inputs;
        match self.phase {
            Phase::Boarding | Phase::Arrived => None,
            Phase::Pushback => {
                self.pushback(p, dt);
                None
            }
            Phase::TaxiOut => {
                let from = p.pushback_length();
                if self.taxi(dt, f64::from(self.taxi_out), from, inputs.taxi_out_length) {
                    self.enter(Phase::Queue);
                    self.speed = 0.0;
                }
                None
            }
            Phase::Queue => {
                if clearance(Request::Takeoff) {
                    if p.lineup_length() > 0.0 {
                        self.enter(Phase::LineUp);
                        self.speed = LINEUP_SPEED;
                    } else {
                        self.enter(Phase::TakeoffRoll);
                    }
                }
                None
            }
            Phase::LineUp => {
                self.speed = LINEUP_SPEED;
                self.along += LINEUP_SPEED * dt;
                if self.along >= p.lineup_length() - 0.01 {
                    self.enter(Phase::TakeoffRoll);
                    self.speed = 0.0;
                }
                None
            }
            Phase::Holding => self.holding(p, dt, clearance),
            Phase::TakeoffRoll => self.takeoff_roll(p, dt),
            Phase::Climb => self.climb(p, dt),
            Phase::Cruise => self.cruise(p, dt),
            Phase::Descent => self.descent(p, dt, clearance),
            Phase::Approach => self.approach(p, dt),
            Phase::Landing => {
                let length = p.landing_length();
                self.speed = (self.speed - BRAKING * dt).max(p.exit_speed());
                self.along = (self.along + self.speed * dt).min(length);
                if self.along >= length - 0.01 {
                    self.enter(Phase::TaxiIn);
                }
                None
            }
            Phase::TaxiIn => {
                if !self.taxi(dt, f64::from(self.taxi_in), 0.0, inputs.taxi_in_length) {
                    return None;
                }
                self.enter(Phase::Arrived);
                self.speed = 0.0;
                Some(Event::Arrived)
            }
        }
    }

    /// Moves along a taxi route from `from` to `length` m, covered in
    /// `total` seconds; true when it is done.
    fn taxi(&mut self, dt: f64, total: f64, from: f64, length: f64) -> bool {
        self.elapsed += dt;
        if total <= 0.0 || self.elapsed >= total {
            self.along = length;
            return true;
        }
        let span = (length - from).max(0.0);
        self.speed = span / total;
        self.along = from + span * self.elapsed / total;
        false
    }

    /// Pushes back along the lead-in, then starts engines; the taxi-out
    /// continues from where the pushback ended.
    fn pushback(&mut self, p: &Profile, dt: f64) {
        let length = p.pushback_length();
        if self.along < length {
            self.along = (self.along + PUSH_SPEED * dt).min(length);
            // Stopped once pushed clear of the stand.
            self.speed = if self.along < length { PUSH_SPEED } else { 0.0 };
            return;
        }
        self.speed = 0.0;
        self.elapsed += dt;
        if self.elapsed >= ENGINE_START {
            self.phase = Phase::TaxiOut;
            self.elapsed = 0.0;
        }
    }
}

#[cfg(test)]
mod tests;
