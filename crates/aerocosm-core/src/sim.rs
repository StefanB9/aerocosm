//! The simulation and its fixed-step loop.

use anyhow::{Context, Result};

use crate::{
    Id, RngSeed, SimDuration, SimTime,
    ops::{
        ActiveRunway, BoardEntry, BoardKind, FlightView, Operations, StartMode, Statistics,
        TaxiRoute,
    },
    scenario::Scenario,
    schedule::{Schedule, ScheduleConfig, Tail, generate},
    world::{Airport, World},
};

/// Length of one simulation step.
pub const TICK: SimDuration = SimDuration::from_secs(1);

/// Parameters that, together with the inputs, fully determine a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    start: SimTime,
    seed: RngSeed,
    start_mode: StartMode,
}

impl SimConfig {
    /// A run starting at `start` with root random seed `seed`, warm-started.
    pub const fn new(start: SimTime, seed: RngSeed) -> Self {
        Self {
            start,
            seed,
            start_mode: StartMode::Warm,
        }
    }

    /// The same config with another start mode (D44).
    #[must_use]
    pub const fn with_start_mode(self, start_mode: StartMode) -> Self {
        Self { start_mode, ..self }
    }

    /// How traffic is placed at the start.
    pub const fn start_mode(self) -> StartMode {
        self.start_mode
    }
}

/// The simulated world, its traffic and its clock.
///
/// Advances only in whole [`TICK`]s. Subsystems run in a fixed order inside
/// [`Self::step`] and draw randomness from their own streams of [`Self::seed`],
/// so identical configs and inputs always produce identical states.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulation {
    scenario: Scenario,
    operations: Option<Operations>,
    time: SimTime,
    seed: RngSeed,
    ticks: u64,
}

impl Simulation {
    /// A simulation of `scenario` at the start of its run. With traffic, the
    /// weekly schedule is generated from the seed, with zone offsets taken at
    /// the start (D45), and the tails are placed by the start mode.
    pub fn new(config: SimConfig, scenario: Scenario) -> Result<Self> {
        let operations = match scenario.schedule_inputs() {
            Some(inputs) => {
                let schedule = generate(&inputs, ScheduleConfig::new(config.seed, config.start))
                    .context("cannot generate the schedule")?;
                Some(
                    Operations::new(
                        &scenario,
                        schedule,
                        config.start,
                        config.seed,
                        config.start_mode,
                    )
                    .context("cannot start flight operations")?,
                )
            }
            None => None,
        };
        Ok(Self {
            scenario,
            operations,
            time: config.start,
            seed: config.seed,
            ticks: 0,
        })
    }

    /// The static world being simulated.
    pub const fn world(&self) -> &World {
        self.scenario.world()
    }

    /// The simulated inputs.
    pub const fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// The weekly schedule being flown, if the scenario has traffic.
    pub fn schedule(&self) -> Option<&Schedule> {
        self.operations.as_ref().map(Operations::schedule)
    }

    /// Every boarding or moving flight.
    pub fn flights(&self) -> impl Iterator<Item = FlightView> + '_ {
        self.operations.iter().flat_map(Operations::flights)
    }

    /// The taxi route of `tail` while it pushes back or taxies at a mapped
    /// airport (plan M4 D86).
    pub fn taxi_route(&self, tail: Id<Tail>) -> Option<TaxiRoute> {
        self.operations.as_ref()?.taxi_route(tail.as_usize())
    }

    /// Up to `count` of `airport`'s departures or arrivals by scheduled time,
    /// with their status: those not yet gone, and finished ones from the last
    /// quarter of an hour; empty without traffic.
    pub fn board(&self, airport: Id<Airport>, kind: BoardKind, count: usize) -> Vec<BoardEntry> {
        self.operations
            .as_ref()
            .map_or_else(Vec::new, |operations| {
                operations.board(airport, kind, count)
            })
    }

    /// The runway ends `airport` uses now, for landing and take-off (plan M3
    /// D55): its operating mode for the prevailing wind, or the best-aligned
    /// runway.
    pub fn runways_in_use(&self, airport: Id<Airport>) -> Vec<ActiveRunway> {
        let modes = self
            .world()
            .airport(airport)
            .zip(self.scenario.traffic())
            .map_or(&[][..], |(place, traffic)| {
                traffic.targets().runway_modes(place.ident())
            });
        crate::ops::active_runways_now(self.world().runways_of(airport), modes)
    }

    /// Punctuality, delays, holding and movements since the start; `None`
    /// without traffic.
    pub fn statistics(&self) -> Option<&Statistics> {
        self.operations.as_ref().map(Operations::statistics)
    }

    /// Aircraft currently off the ground.
    pub fn airborne(&self) -> usize {
        self.operations.as_ref().map_or(0, Operations::airborne)
    }

    /// Take-offs plus landings at `airport` since the start.
    pub fn movements(&self, airport: Id<Airport>) -> u32 {
        self.operations
            .as_ref()
            .map_or(0, |operations| operations.movements(airport))
    }

    /// Flights completed (on blocks) since the start.
    pub fn flights_flown(&self) -> u64 {
        self.operations
            .as_ref()
            .map_or(0, Operations::flights_flown)
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

    /// Advances by one tick. A step that would pass the end of time fails
    /// without changing anything; a failure inside the subsystems leaves the
    /// run unusable.
    pub fn step(&mut self) -> Result<()> {
        let next = self
            .time
            .checked_add(TICK)
            .with_context(|| format!("cannot advance simulation past {}", self.time))?;
        if let Some(operations) = &mut self.operations {
            operations
                .step(&self.scenario)
                .context("flight operations failed")?;
        }
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
    use crate::{
        test_data::data,
        world::{FORMAT_VERSION, WorldDefinition},
    };

    fn start() -> Result<SimTime> {
        "2026-11-05T00:00:00Z".parse()
    }

    fn simulation() -> Result<Simulation> {
        Simulation::new(
            SimConfig::new(start()?, RngSeed::new(42)),
            Scenario::empty(),
        )
    }

    fn germany(mode: StartMode) -> Result<Simulation> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        let config =
            SimConfig::new("2026-07-08T10:00:00Z".parse()?, RngSeed::new(9)).with_start_mode(mode);
        Simulation::new(config, scenario)
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
            countries: Vec::new(),
            airports: Vec::new(),
            runways: Vec::new(),
        }
        .to_json_pretty()?;
        let sim = Simulation::new(
            SimConfig::new(start()?, RngSeed::new(1)),
            Scenario::new(World::from_json(&json)?),
        )?;
        assert_eq!(sim.world().airport_count(), 0);
        assert!(sim.schedule().is_none());
        assert_eq!(sim.flights().count(), 0);
        assert_eq!((sim.airborne(), sim.flights_flown()), (0, 0));
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
    fn test_simulation_step_at_end_of_time_fails_cleanly() -> Result<()> {
        let mut sim = Simulation::new(
            SimConfig::new(SimTime::MAX, RngSeed::new(0)),
            Scenario::empty(),
        )?;
        let before = sim.clone();
        assert!(sim.step().is_err());
        assert_eq!(sim, before, "a failed step must not change state");
        Ok(())
    }

    #[test]
    fn test_sim_config_start_mode_defaults_to_warm() -> Result<()> {
        let config = SimConfig::new(start()?, RngSeed::new(1));
        assert_eq!(config.start_mode(), StartMode::Warm);
        assert_eq!(
            config.with_start_mode(StartMode::Cold).start_mode(),
            StartMode::Cold
        );
        Ok(())
    }

    #[test]
    fn test_simulation_with_traffic_flies_the_schedule() -> Result<()> {
        let mut sim = germany(StartMode::Warm)?;
        let schedule = sim.schedule().context("schedule")?;
        assert!(schedule.flights().len() > 50_000);
        assert!(sim.airborne() > 300);
        sim.run_ticks(600)?;
        let airborne = sim.flights().filter(|f| f.phase().is_airborne()).count();
        assert_eq!(airborne, sim.airborne());
        assert!(sim.flights_flown() > 0);
        let frankfurt = sim
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        assert!(sim.movements(frankfurt) > 0);
        Ok(())
    }

    #[test]
    fn test_simulation_reports_statistics_with_traffic_only() -> Result<()> {
        let mut sim = germany(StartMode::Warm)?;
        sim.run_ticks(600)?;
        let stats = sim.statistics().context("statistics")?;
        assert!(stats.network().departures() > 0);
        assert!(simulation()?.statistics().is_none());
        Ok(())
    }

    #[test]
    fn test_simulation_lists_runways_in_use() -> Result<()> {
        let sim = germany(StartMode::Warm)?;
        let frankfurt = sim
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let active = sim.runways_in_use(frankfurt);
        let runways = sim.world().runways_of(frankfurt);
        let name = |a: &ActiveRunway| runways[a.runway].ends()[a.end].designator().to_string();
        let landing: Vec<String> = active.iter().filter(|a| a.arrivals).map(name).collect();
        let departing: Vec<String> = active.iter().filter(|a| a.departures).map(name).collect();
        assert_eq!(landing.len(), 2, "{landing:?}");
        assert!(departing.contains(&"18".to_owned()), "{departing:?}");
        Ok(())
    }

    #[test]
    fn test_simulation_cold_start_has_nothing_airborne() -> Result<()> {
        let sim = germany(StartMode::Cold)?;
        assert_eq!(sim.airborne(), 0);
        assert!(sim.schedule().is_some());
        Ok(())
    }

    #[test]
    fn test_simulation_with_traffic_is_deterministic() -> Result<()> {
        let mut a = germany(StartMode::Warm)?;
        let mut b = germany(StartMode::Warm)?;
        a.run_ticks(300)?;
        b.run_ticks(300)?;
        assert_eq!(a, b);
        Ok(())
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
            let mut sim = Simulation::new(SimConfig::new(start, RngSeed::new(1)), Scenario::empty())
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
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
