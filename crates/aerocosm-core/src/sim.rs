//! The simulation and its fixed-step loop.

use anyhow::{Context, Result};

use crate::{RngSeed, SimDuration, SimTime, world::World};

/// Length of one simulation step.
pub const TICK: SimDuration = SimDuration::from_secs(1);

/// Parameters that, together with the inputs, fully determine a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    start: SimTime,
    seed: RngSeed,
}

impl SimConfig {
    /// A run starting at `start` with root random seed `seed`.
    pub const fn new(start: SimTime, seed: RngSeed) -> Self {
        Self { start, seed }
    }
}

/// The simulated world and its clock.
///
/// Advances only in whole [`TICK`]s. Subsystems run in a fixed order inside
/// [`Self::step`] and draw randomness from their own streams of [`Self::seed`],
/// so identical configs and inputs always produce identical states.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulation {
    world: World,
    time: SimTime,
    seed: RngSeed,
    ticks: u64,
}

impl Simulation {
    /// A simulation of `world` at the start of its run.
    pub const fn new(config: SimConfig, world: World) -> Self {
        Self {
            world,
            time: config.start,
            seed: config.seed,
            ticks: 0,
        }
    }

    /// The static world being simulated.
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// The current simulated time.
    pub const fn time(&self) -> SimTime {
        self.time
    }

    /// Ticks run so far.
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The root random seed.
    pub const fn seed(&self) -> RngSeed {
        self.seed
    }

    /// Advances by one tick. On error the state is unchanged.
    pub fn step(&mut self) -> Result<()> {
        let next = self
            .time
            .checked_add(TICK)
            .with_context(|| format!("cannot advance simulation past {}", self.time))?;
        // Subsystems will run here, in a fixed order, before time advances.
        self.time = next;
        self.ticks += 1;
        Ok(())
    }

    /// Advances by `count` ticks.
    pub fn run_ticks(&mut self, count: u32) -> Result<()> {
        for _ in 0..count {
            self.step()?;
        }
        Ok(())
    }

    /// Advances by whole ticks while the next tick ends at or before `end`;
    /// returns the number of ticks run.
    pub fn run_until(&mut self, end: SimTime) -> Result<u64> {
        let mut ran = 0;
        while end.duration_since(self.time) >= TICK {
            self.step()?;
            ran += 1;
        }
        Ok(ran)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::world::{FORMAT_VERSION, WorldDefinition};

    fn start() -> Result<SimTime> {
        "2026-11-05T00:00:00Z".parse()
    }

    fn simulation() -> Result<Simulation> {
        Ok(Simulation::new(
            SimConfig::new(start()?, RngSeed::new(42)),
            World::empty(),
        ))
    }

    #[test]
    fn test_simulation_new_starts_at_config() -> Result<()> {
        let sim = simulation()?;
        assert_eq!(sim.time(), start()?);
        assert_eq!(sim.ticks(), 0);
        assert_eq!(sim.seed(), RngSeed::new(42));
        Ok(())
    }

    #[test]
    fn test_simulation_owns_world() -> Result<()> {
        let json = WorldDefinition {
            format_version: FORMAT_VERSION,
            airports: Vec::new(),
            runways: Vec::new(),
        }
        .to_json_pretty()?;
        let sim = Simulation::new(
            SimConfig::new(start()?, RngSeed::new(1)),
            World::from_json(&json)?,
        );
        assert_eq!(sim.world().airport_count(), 0);
        Ok(())
    }

    #[test]
    fn test_simulation_step_advances_one_tick() -> Result<()> {
        let mut sim = simulation()?;
        sim.step()?;
        assert_eq!(sim.time(), start()?.checked_add(TICK)?);
        assert_eq!(sim.ticks(), 1);
        Ok(())
    }

    #[test]
    fn test_simulation_run_ticks() -> Result<()> {
        let mut sim = simulation()?;
        sim.run_ticks(3600)?;
        assert_eq!(
            sim.time(),
            start()?.checked_add(SimDuration::from_hours(1))?
        );
        assert_eq!(sim.ticks(), 3600);
        Ok(())
    }

    #[test]
    fn test_simulation_run_until_stops_on_last_whole_tick() -> Result<()> {
        let mut sim = simulation()?;
        let end = start()?.checked_add(SimDuration::from_millis(10_500))?;
        assert_eq!(sim.run_until(end)?, 10);
        assert_eq!(
            sim.time(),
            start()?.checked_add(SimDuration::from_secs(10))?
        );
        Ok(())
    }

    #[test]
    fn test_simulation_run_until_past_is_noop() -> Result<()> {
        let mut sim = simulation()?;
        let past = start()?.checked_sub(SimDuration::from_hours(1))?;
        assert_eq!(sim.run_until(past)?, 0);
        assert_eq!(sim.time(), start()?);
        Ok(())
    }

    #[test]
    fn test_simulation_step_at_end_of_time_fails_cleanly() {
        let mut sim = Simulation::new(
            SimConfig::new(SimTime::MAX, RngSeed::new(0)),
            World::empty(),
        );
        let before = sim.clone();
        assert!(sim.step().is_err());
        assert_eq!(sim, before, "a failed step must not change state");
    }

    #[test]
    fn test_simulation_is_deterministic() -> Result<()> {
        let mut a = simulation()?;
        let mut b = simulation()?;
        a.run_ticks(10_000).context("run a")?;
        b.run_ticks(10_000).context("run b")?;
        assert_eq!(a, b);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_simulation_run_until_lands_within_one_tick(offset_ms in 0_i64..86_400_000) {
            let start = start().map_err(|e| TestCaseError::fail(e.to_string()))?;
            let mut sim = Simulation::new(SimConfig::new(start, RngSeed::new(1)), World::empty());
            let end = start
                .checked_add(SimDuration::from_millis(offset_ms))
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
            let ticks = sim.run_until(end).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(ticks, sim.ticks());
            prop_assert!(sim.time() <= end);
            prop_assert!(end.duration_since(sim.time()) < TICK);
        }
    }
}
