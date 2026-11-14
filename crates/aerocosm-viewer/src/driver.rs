//! Runs the simulation from the viewer's frame loop at the chosen speed.

use std::time::Duration;

use aerocosm_core::{Pacer, SimSpeed, SimTime, Simulation, TICK, WallClock, WallInstant};
use anyhow::{Context, Result};
use bevy::ecs::{resource::Resource, schedule::SystemSet};

/// At most this many ticks per frame; `Max` speed runs exactly this many.
const MAX_TICKS_PER_FRAME: u32 = 2000;

/// Systems that advance the simulation; readers of its state run after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SimSet;

/// Systems that read the simulation: they run once it exists, after the
/// ground has loaded (plan M5 D132).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SimReady;

/// The simulation plus the pacer that decides how far to advance it each frame.
#[derive(Resource)]
pub(crate) struct SimDriver {
    simulation: Simulation,
    pacer: Pacer,
}

/// A wall clock reading taken from Bevy's real-time clock for one frame.
struct FrameClock(Duration);

impl WallClock for FrameClock {
    fn now(&self) -> WallInstant {
        WallInstant::from_elapsed(self.0)
    }
}

impl SimDriver {
    /// Drives `simulation` at the default speed.
    pub(crate) fn new(simulation: Simulation) -> Result<Self> {
        Ok(Self {
            simulation,
            pacer: Pacer::new(TICK, MAX_TICKS_PER_FRAME)?,
        })
    }

    /// The simulation, for reading state.
    pub(crate) const fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// The current speed.
    pub(crate) const fn speed(&self) -> SimSpeed {
        self.pacer.speed()
    }

    /// How far real time has progressed into the next tick, `0..1` (D50).
    pub(crate) fn tick_fraction(&self) -> f64 {
        self.pacer.tick_fraction()
    }

    /// Changes the speed from the next frame on.
    pub(crate) const fn set_speed(&mut self, speed: SimSpeed) {
        self.pacer.set_speed(speed);
    }

    /// Runs `count` ticks regardless of speed.
    #[cfg(test)]
    pub(crate) fn run_ticks(&mut self, count: u32) -> Result<()> {
        self.simulation.run_ticks(count)
    }

    /// Runs the ticks due by `elapsed`, the real time since the app started.
    pub(crate) fn advance(&mut self, elapsed: Duration) -> Result<()> {
        let ticks = self.pacer.ticks_due(&FrameClock(elapsed));
        self.simulation
            .run_ticks(ticks)
            .with_context(|| format!("simulation stopped at {}", self.simulation.time()))
    }
}

/// A friendly clock such as `Thu 5 Nov 2026  06:07:08 UTC`.
pub(crate) fn format_clock(time: SimTime) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = time.as_millis().div_euclid(86_400_000);
    // 1970-01-01 was a Thursday.
    let weekday = WEEKDAYS[usize::try_from((days + 4).rem_euclid(7)).unwrap_or_default()];
    let civil = time.to_civil();
    let month = MONTHS[usize::from(civil.month().saturating_sub(1)).min(11)];
    format!(
        "{weekday} {} {month} {}  {:02}:{:02}:{:02} UTC",
        civil.day(),
        civil.year(),
        civil.hour(),
        civil.minute(),
        civil.second()
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aerocosm_core::{RngSeed, Scenario, SimConfig, SimSpeed, SimTime, Simulation};
    use anyhow::Result;

    use super::*;

    fn driver() -> Result<SimDriver> {
        let start: SimTime = "2026-11-05T06:00:00Z".parse()?;
        SimDriver::new(Simulation::new(
            SimConfig::new(start, RngSeed::new(1)),
            Scenario::empty(),
        )?)
    }

    #[test]
    fn test_driver_runs_at_default_speed() -> Result<()> {
        let mut driver = driver()?;
        assert_eq!(driver.speed(), SimSpeed::X60);
        driver.advance(Duration::ZERO)?;
        driver.advance(Duration::from_secs(1))?;
        assert_eq!(driver.simulation().ticks(), 60);
        Ok(())
    }

    #[test]
    fn test_driver_pause_and_speed_change() -> Result<()> {
        let mut driver = driver()?;
        driver.advance(Duration::ZERO)?;
        driver.set_speed(SimSpeed::Paused);
        driver.advance(Duration::from_secs(5))?;
        assert_eq!(driver.simulation().ticks(), 0);
        driver.set_speed(SimSpeed::X10);
        driver.advance(Duration::from_secs(6))?;
        assert_eq!(driver.simulation().ticks(), 10);
        Ok(())
    }

    #[test]
    fn test_format_clock() -> Result<()> {
        let time: SimTime = "2026-11-05T06:07:08Z".parse()?;
        assert_eq!(format_clock(time), "Thu 5 Nov 2026  06:07:08 UTC");
        let sunday: SimTime = "1970-01-04T23:59:59Z".parse()?;
        assert_eq!(format_clock(sunday), "Sun 4 Jan 1970  23:59:59 UTC");
        Ok(())
    }
}
