//! The airborne phases: take-off roll to approach, holding.

use super::{
    Event, FINAL_LENGTH, GLIDE_SLOPE, Motion, Phase, Profile, Request, TAKEOFF_ACCELERATION,
};

impl Motion {
    /// Holding: descends to the hold level, leaves when cleared.
    pub(super) fn holding(
        &mut self,
        p: &Profile,
        dt: f64,
        clearance: &mut impl FnMut(Request) -> bool,
    ) -> Option<Event> {
        let inputs = &p.inputs;
        self.elapsed += dt;
        self.speed = crate::ops::hold::HOLD_SPEED;
        if self.altitude > self.hold_altitude {
            self.altitude = (self.altitude - inputs.descent_rate * dt).max(self.hold_altitude);
        }
        if clearance(Request::LeaveHold) {
            self.hold_at = None;
            self.phase = Phase::Descent;
        }
        None
    }

    /// Take-off roll: accelerates to rotation.
    pub(super) fn takeoff_roll(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        self.speed += TAKEOFF_ACCELERATION * dt;
        self.along += self.speed * dt;
        if self.speed < p.rotate_speed {
            return None;
        }
        self.enter(Phase::Climb);
        self.speed = p.climb_entry_speed;
        Some(Event::Takeoff)
    }

    /// Climb towards the leg's cruise altitude.
    pub(super) fn climb(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        let inputs = &p.inputs;
        let span = (p.cruise_altitude - inputs.origin_elevation).max(1.0);
        let progress = ((self.altitude - inputs.origin_elevation) / span).clamp(0.0, 1.0);
        self.speed = p.climb_entry_speed + (inputs.cruise_speed - p.climb_entry_speed) * progress;
        if let Some(at) = self.hold_due(self.speed * dt) {
            self.enter_hold(at);
            return None;
        }
        self.along += self.speed * dt;
        self.altitude = (self.altitude + inputs.climb_rate * dt).min(p.cruise_altitude);
        let remaining = inputs.enroute_length - self.along;
        if remaining <= p.descent_distance(self.altitude, self.speed) {
            self.phase = Phase::Descent;
        } else if self.altitude >= p.cruise_altitude {
            self.phase = Phase::Cruise;
        }
        None
    }

    /// Level cruise until the top of descent.
    pub(super) fn cruise(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        let inputs = &p.inputs;
        self.speed = inputs.cruise_speed * self.speed_factor;
        if let Some(at) = self.hold_due(self.speed * dt) {
            self.enter_hold(at);
            return None;
        }
        self.along += self.speed * dt;
        let remaining = inputs.enroute_length - self.along;
        if remaining <= p.descent_distance(self.altitude, self.speed) {
            self.phase = Phase::Descent;
        }
        None
    }

    /// Straight-path descent to the final fix; orbits there without approach
    /// clearance.
    pub(super) fn descent(
        &mut self,
        p: &Profile,
        dt: f64,
        clearance: &mut impl FnMut(Request) -> bool,
    ) -> Option<Event> {
        let inputs = &p.inputs;
        let span = (p.cruise_altitude - p.faf_altitude).max(1.0);
        let progress = ((self.altitude - p.faf_altitude) / span).clamp(0.0, 1.0);
        self.speed = (p.descent_exit_speed
            + (inputs.cruise_speed - p.descent_exit_speed) * progress)
            * self.speed_factor;
        let step = self.speed * dt;
        if let Some(at) = self.hold_due(step) {
            self.enter_hold(at);
            return None;
        }
        let remaining = inputs.enroute_length - self.along;
        if step >= remaining {
            if clearance(Request::Approach) {
                self.enter(Phase::Approach);
                self.altitude = p.faf_altitude;
                self.speed = inputs.approach_speed;
            } else {
                // Not yet: orbit at the fix at the current level.
                self.hold_altitude = self.altitude.max(p.faf_altitude);
                self.enter_hold(inputs.enroute_length);
            }
        } else {
            // A straight path down to the fix: the rate follows the
            // geometry, which keeps the arrival altitude exact.
            self.altitude -= (self.altitude - p.faf_altitude).max(0.0) * step / remaining;
            self.along += step;
        }
        None
    }

    /// The 3° final down to the threshold.
    pub(super) fn approach(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        let inputs = &p.inputs;
        self.speed = inputs.approach_speed;
        self.along += self.speed * dt;
        if self.along < FINAL_LENGTH {
            self.altitude =
                inputs.destination_elevation + (FINAL_LENGTH - self.along) * GLIDE_SLOPE;
            return None;
        }
        self.enter(Phase::Landing);
        self.altitude = inputs.destination_elevation;
        Some(Event::Landing)
    }
}
