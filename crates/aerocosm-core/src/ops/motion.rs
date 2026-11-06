//! Per-leg kinematics on scalars (D36, D48): distance along the current
//! segment, altitude and speed, advanced without trigonometry.

use super::Phase;

/// Take-off acceleration along the runway, m/s².
const TAKEOFF_ACCELERATION: f64 = 2.0;
/// Braking deceleration after touchdown, m/s².
const BRAKING: f64 = 1.8;
/// Speed at which an aircraft leaves the runway, m/s (about 15 kt).
const TAXI_SPEED: f64 = 8.0;
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
    inputs: ProfileInputs,
    rotate_speed: f64,
    climb_entry_speed: f64,
    descent_exit_speed: f64,
    cruise_altitude: f64,
    faf_altitude: f64,
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
    fn descent_distance(&self, altitude: f64, speed: f64) -> f64 {
        (altitude - self.faf_altitude).max(0.0) / self.inputs.descent_rate
            * (speed + self.descent_exit_speed)
            / 2.0
    }
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
    taxi_out: f64,
    taxi_in: f64,
}

impl Motion {
    /// Pushes back at the origin gate with the given taxi times, s.
    pub(crate) const fn start(profile: &Profile, taxi_out: f64, taxi_in: f64) -> Self {
        Self {
            phase: Phase::TaxiOut,
            along: 0.0,
            altitude: profile.inputs.origin_elevation,
            speed: 0.0,
            elapsed: 0.0,
            taxi_out,
            taxi_in,
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
        }
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
    pub(crate) fn advance(&mut self, p: &Profile, dt: f64) -> Option<Event> {
        let inputs = &p.inputs;
        match self.phase {
            Phase::Boarding | Phase::Arrived => None,
            Phase::TaxiOut => {
                if self.taxi(dt, self.taxi_out, inputs.taxi_out_length) {
                    self.enter(Phase::TakeoffRoll);
                    self.speed = 0.0;
                }
                None
            }
            Phase::TakeoffRoll => {
                self.speed += TAKEOFF_ACCELERATION * dt;
                self.along += self.speed * dt;
                if self.speed < p.rotate_speed {
                    return None;
                }
                self.enter(Phase::Climb);
                self.speed = p.climb_entry_speed;
                Some(Event::Takeoff)
            }
            Phase::Climb => {
                let span = (p.cruise_altitude - inputs.origin_elevation).max(1.0);
                let progress = ((self.altitude - inputs.origin_elevation) / span).clamp(0.0, 1.0);
                self.speed =
                    p.climb_entry_speed + (inputs.cruise_speed - p.climb_entry_speed) * progress;
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
            Phase::Cruise => {
                self.speed = inputs.cruise_speed;
                self.along += self.speed * dt;
                let remaining = inputs.enroute_length - self.along;
                if remaining <= p.descent_distance(self.altitude, self.speed) {
                    self.phase = Phase::Descent;
                }
                None
            }
            Phase::Descent => {
                let span = (p.cruise_altitude - p.faf_altitude).max(1.0);
                let progress = ((self.altitude - p.faf_altitude) / span).clamp(0.0, 1.0);
                self.speed =
                    p.descent_exit_speed + (inputs.cruise_speed - p.descent_exit_speed) * progress;
                let step = self.speed * dt;
                let remaining = inputs.enroute_length - self.along;
                if step >= remaining {
                    self.enter(Phase::Approach);
                    self.altitude = p.faf_altitude;
                    self.speed = inputs.approach_speed;
                } else {
                    // A straight path down to the fix: the rate follows the
                    // geometry, which keeps the arrival altitude exact.
                    self.altitude -= (self.altitude - p.faf_altitude).max(0.0) * step / remaining;
                    self.along += step;
                }
                None
            }
            Phase::Approach => {
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
            Phase::Landing => {
                self.speed = (self.speed - BRAKING * dt).max(TAXI_SPEED);
                self.along = (self.along + self.speed * dt).min(p.landing_length());
                if self.speed <= TAXI_SPEED {
                    self.enter(Phase::TaxiIn);
                }
                None
            }
            Phase::TaxiIn => {
                if !self.taxi(dt, self.taxi_in, inputs.taxi_in_length) {
                    return None;
                }
                self.enter(Phase::Arrived);
                self.speed = 0.0;
                Some(Event::Arrived)
            }
        }
    }

    /// Moves along a straight taxi route of `length` covered in `total`
    /// seconds; true when it is done.
    fn taxi(&mut self, dt: f64, total: f64, length: f64) -> bool {
        self.elapsed += dt;
        if total <= 0.0 || self.elapsed >= total {
            self.along = length;
            return true;
        }
        self.speed = length / total;
        self.along = length * self.elapsed / total;
        false
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use proptest::prelude::*;

    use super::*;

    /// An A320-like profile over `distance` metres between sea-level fields.
    fn profile(distance: f64) -> Profile {
        Profile::new(&ProfileInputs {
            cruise_speed: 230.0,
            cruise_altitude: 11_000.0,
            climb_rate: 10.0,
            descent_rate: 10.0,
            approach_speed: 70.0,
            origin_elevation: 100.0,
            destination_elevation: 450.0,
            enroute_length: distance,
            taxi_out_length: 2_000.0,
            taxi_in_length: 1_500.0,
        })
    }

    struct Flown {
        phases: Vec<Phase>,
        events: Vec<Event>,
        seconds: u32,
        peak: f64,
        faf_altitude: Option<f64>,
    }

    fn fly(profile: &Profile, dt: f64) -> Result<Flown> {
        let mut motion = Motion::start(profile, 600.0, 300.0);
        let mut flown = Flown {
            phases: vec![motion.phase()],
            events: Vec::new(),
            seconds: 0,
            peak: 0.0,
            faf_altitude: None,
        };
        while motion.phase() != Phase::Arrived {
            if flown.seconds > 20 * 3600 {
                bail!("never arrived; stuck in {:?}", motion.phase());
            }
            let before = motion.phase();
            if let Some(event) = motion.advance(profile, dt) {
                flown.events.push(event);
            }
            if before == Phase::Descent && motion.phase() == Phase::Approach {
                flown.faf_altitude = Some(motion.altitude());
            }
            if motion.phase() != before {
                flown.phases.push(motion.phase());
            }
            flown.peak = flown.peak.max(motion.altitude());
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = dt as u32;
            flown.seconds += step;
        }
        Ok(flown)
    }

    #[test]
    fn test_motion_flies_a_full_leg() -> Result<()> {
        let profile = profile(600_000.0);
        let flown = fly(&profile, 1.0)?;
        assert_eq!(
            flown.phases,
            [
                Phase::TaxiOut,
                Phase::TakeoffRoll,
                Phase::Climb,
                Phase::Cruise,
                Phase::Descent,
                Phase::Approach,
                Phase::Landing,
                Phase::TaxiIn,
                Phase::Arrived,
            ]
        );
        assert_eq!(
            flown.events,
            [Event::Takeoff, Event::Landing, Event::Arrived]
        );
        // 600 km at ~230 m/s plus taxi, climb and approach: about 75 minutes.
        assert!((3_600..6_000).contains(&flown.seconds), "{}", flown.seconds);
        let faf = flown.faf_altitude.unwrap_or_default();
        assert!((faf - profile.faf_altitude()).abs() < 30.0, "FAF at {faf}");
        Ok(())
    }

    #[test]
    fn test_motion_caps_altitude_on_short_legs() -> Result<()> {
        let profile = profile(120_000.0);
        assert!(profile.cruise_altitude() < 11_000.0);
        let flown = fly(&profile, 1.0)?;
        assert!(flown.peak <= profile.cruise_altitude() + 1e-6);
        assert!(!flown.phases.contains(&Phase::Cruise) || flown.peak > 3_000.0);
        Ok(())
    }

    #[test]
    fn test_motion_reports_kinematic_state() {
        let profile = profile(300_000.0);
        let mut motion = Motion::start(&profile, 600.0, 300.0);
        assert_eq!(motion.phase(), Phase::TaxiOut);
        assert!((motion.altitude() - 100.0).abs() < 1e-9);
        motion.advance(&profile, 300.0);
        assert!((motion.along() - 1_000.0).abs() < 1e-6, "half the taxi-out");
        assert!((motion.speed() - 2_000.0 / 600.0).abs() < 1e-9);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_motion_always_completes_in_order(
            distance in 40_000.0_f64..4_000_000.0,
            dt in prop::sample::select(vec![1.0, 10.0]),
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let profile = profile(distance);
            let flown = fly(&profile, dt).map_err(fail)?;
            prop_assert!(flown.phases.windows(2).all(|pair| pair[0] < pair[1]));
            prop_assert_eq!(flown.events, vec![Event::Takeoff, Event::Landing, Event::Arrived]);
            prop_assert!(flown.peak <= profile.cruise_altitude() + 1e-6);
            let faf = flown.faf_altitude.unwrap_or_default();
            prop_assert!((faf - profile.faf_altitude()).abs() < 60.0, "FAF at {}", faf);
            // Never much slower than cruising the whole way plus an hour.
            prop_assert!(f64::from(flown.seconds) < distance / 230.0 * 1.3 + 3_600.0);
        }
    }
}
