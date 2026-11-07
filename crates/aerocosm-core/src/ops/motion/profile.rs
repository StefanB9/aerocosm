//! A leg's speeds, altitudes and segment lengths.

use super::{
    BRAKING, CLIMB_ENTRY_FACTOR, DESCENT_EXIT_FACTOR, FINAL_LENGTH, GLIDE_SLOPE, ROTATE_FACTOR,
    TAKEOFF_ACCELERATION, TAXI_SPEED,
};

/// Everything a leg's profile is built from, in SI units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct ProfileInputs {
    /// Cruise true airspeed, m/s.
    pub(crate) cruise_speed: f64,
    /// The type's cruise altitude, m.
    pub(crate) cruise_altitude: f64,
    /// Climb rate, m/s.
    pub(crate) climb_rate: f64,
    /// Descent rate, m/s.
    pub(crate) descent_rate: f64,
    /// Final approach speed, m/s.
    pub(crate) approach_speed: f64,
    /// Field elevation at the origin, m.
    pub(crate) origin_elevation: f64,
    /// Field elevation at the destination, m.
    pub(crate) destination_elevation: f64,
    /// Great-circle length from lift-off to the final approach fix, m.
    pub(crate) enroute_length: f64,
    /// Straight-line taxi length at the origin, m (D47).
    pub(crate) taxi_out_length: f64,
    /// Straight-line taxi length at the destination, m (D47).
    pub(crate) taxi_in_length: f64,
    /// Length of the pushback at the start of the taxi-out, m; 0 without
    /// one (plan M4 D72).
    pub(crate) pushback_length: f64,
    /// Distance from the threshold to the planned exit, m; 0 to stop where
    /// braking reaches taxi speed (plan M4 D71).
    pub(crate) landing_length: f64,
    /// Speed at which the runway is left, m/s; 0 for taxi speed.
    pub(crate) exit_speed: f64,
    /// From the holding position onto the runway, m; 0 when the roll starts
    /// where the aircraft queued (plan M4 D78).
    pub(crate) lineup_length: f64,
}

/// A leg's precomputed speeds, altitudes and segment lengths.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Profile {
    pub(super) inputs: ProfileInputs,
    pub(super) rotate_speed: f64,
    pub(super) climb_entry_speed: f64,
    pub(super) descent_exit_speed: f64,
    pub(super) cruise_altitude: f64,
    pub(super) faf_altitude: f64,
}

impl Profile {
    /// Derives the profile; the cruise altitude is lowered on short legs so
    /// that climb and descent fit between lift-off and the approach fix.
    pub(crate) fn new(inputs: &ProfileInputs) -> Self {
        let climb_entry_speed = inputs.approach_speed * CLIMB_ENTRY_FACTOR;
        let descent_exit_speed = inputs.approach_speed * DESCENT_EXIT_FACTOR;
        let faf_altitude = inputs.destination_elevation + FINAL_LENGTH * GLIDE_SLOPE;
        // Ground distance per metre of climb and of descent at average speeds.
        let per_climb = f64::midpoint(climb_entry_speed, inputs.cruise_speed) / inputs.climb_rate;
        let per_descent =
            f64::midpoint(descent_exit_speed, inputs.cruise_speed) / inputs.descent_rate;
        let fits = (inputs.enroute_length
            + inputs.origin_elevation * per_climb
            + faf_altitude * per_descent)
            / (per_climb + per_descent);
        let floor = faf_altitude.max(inputs.origin_elevation);
        let cruise_altitude = fits.min(inputs.cruise_altitude).max(floor);
        Self {
            inputs: *inputs,
            rotate_speed: inputs.approach_speed * ROTATE_FACTOR,
            climb_entry_speed,
            descent_exit_speed,
            cruise_altitude,
            faf_altitude,
        }
    }

    /// The leg's cruise altitude, m.
    #[cfg(test)]
    pub(crate) const fn cruise_altitude(&self) -> f64 {
        self.cruise_altitude
    }

    /// Altitude at the final approach fix, m.
    #[cfg(test)]
    pub(crate) const fn faf_altitude(&self) -> f64 {
        self.faf_altitude
    }

    /// Speed at the start of the climb, m/s.
    pub(crate) const fn climb_entry_speed(&self) -> f64 {
        self.climb_entry_speed
    }

    /// Final approach speed, m/s.
    pub(crate) const fn approach_speed(&self) -> f64 {
        self.inputs.approach_speed
    }

    /// Length of the en-route part from lift-off to the final fix, m.
    pub(crate) const fn enroute_length(&self) -> f64 {
        self.inputs.enroute_length
    }

    /// Runway length used by the take-off roll, m.
    pub(crate) fn takeoff_length(&self) -> f64 {
        self.rotate_speed * self.rotate_speed / (2.0 * TAKEOFF_ACCELERATION)
    }

    /// Runway length used by the landing roll, m: to the planned exit, or
    /// where braking reaches taxi speed.
    pub(crate) fn landing_length(&self) -> f64 {
        if self.inputs.landing_length > 0.0 {
            return self.inputs.landing_length;
        }
        let approach = self.inputs.approach_speed;
        (approach * approach - TAXI_SPEED * TAXI_SPEED).max(0.0) / (2.0 * BRAKING)
    }

    /// Speed at which the runway is left, m/s.
    pub(crate) fn exit_speed(&self) -> f64 {
        if self.inputs.exit_speed > 0.0 {
            self.inputs.exit_speed
        } else {
            TAXI_SPEED
        }
    }

    /// Sets the taxi-out length once ground control has routed it, m.
    pub(crate) const fn set_taxi_out_length(&mut self, length: f64) {
        self.inputs.taxi_out_length = length;
    }

    /// Taxi-out length, pushback included, m.
    pub(crate) const fn taxi_out_length(&self) -> f64 {
        self.inputs.taxi_out_length
    }

    /// Sets the landing roll's length when it ends at another exit, m.
    pub(crate) const fn set_landing_length(&mut self, length: f64) {
        self.inputs.landing_length = length;
    }

    /// Sets the taxi-in length once the route to the stand is known, m.
    pub(crate) const fn set_taxi_in_length(&mut self, length: f64) {
        self.inputs.taxi_in_length = length;
    }

    /// Length of the line-up, m.
    pub(crate) const fn lineup_length(&self) -> f64 {
        self.inputs.lineup_length
    }

    /// Length of the pushback, m.
    pub(crate) const fn pushback_length(&self) -> f64 {
        self.inputs.pushback_length
    }

    /// Ground distance needed to descend from `altitude` at `speed` to the
    /// approach fix.
    pub(super) fn descent_distance(&self, altitude: f64, speed: f64) -> f64 {
        (altitude - self.faf_altitude).max(0.0) / self.inputs.descent_rate
            * (speed + self.descent_exit_speed)
            / 2.0
    }
}
