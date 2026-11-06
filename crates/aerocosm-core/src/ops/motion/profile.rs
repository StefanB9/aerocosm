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

    /// Runway length used by the take-off roll, m.
    pub(crate) fn takeoff_length(&self) -> f64 {
        self.rotate_speed * self.rotate_speed / (2.0 * TAKEOFF_ACCELERATION)
    }

    /// Runway length used by the landing roll, m.
    pub(crate) fn landing_length(&self) -> f64 {
        let approach = self.inputs.approach_speed;
        (approach * approach - TAXI_SPEED * TAXI_SPEED).max(0.0) / (2.0 * BRAKING)
    }

    /// Ground distance needed to descend from `altitude` at `speed` to the
    /// approach fix.
    pub(super) fn descent_distance(&self, altitude: f64, speed: f64) -> f64 {
        (altitude - self.faf_altitude).max(0.0) / self.inputs.descent_rate
            * (speed + self.descent_exit_speed)
            / 2.0
    }
}
