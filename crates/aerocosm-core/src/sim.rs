//! The simulation and its fixed-step loop.

use anyhow::{Context, Result};

use crate::{
    Id, RngSeed, SimDuration, SimTime,
    ops::{
        ActiveRunway, Airframe, BoardEntry, BoardKind, FlightView, Operations, StartMode,
        Statistics, TaxiRoute,
    },
    scenario::Scenario,
    schedule::{Schedule, ScheduleConfig, generate},
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

    /// The flight `airframe` is on, if it is boarding or moving: the view
    /// [`Simulation::flights`] lists for it, one aircraft at a time, so that
    /// callers can build views in parallel (plan M9 PR 2).
    pub fn flight(&self, airframe: Id<Airframe>) -> Option<FlightView> {
        self.operations.as_ref()?.flight_view(airframe.as_usize())
    }

    /// The taxi route of `airframe` while it pushes back or taxies at a
    /// mapped airport (plan M4 D86).
    pub fn taxi_route(&self, airframe: Id<Airframe>) -> Option<TaxiRoute> {
        self.operations.as_ref()?.taxi_route(airframe.as_usize())
    }

    /// How many aircraft fly the schedule (plan M8 D169); 0 without
    /// traffic.
    pub fn airframe_count(&self) -> usize {
        self.operations
            .as_ref()
            .map_or(0, Operations::airframe_count)
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
        self.world()
            .airport(airport)
            .map_or_else(Vec::new, |place| {
                crate::ops::active_runways_now(place, self.world().runways_of(airport), modes)
            })
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

    /// Advances by one tick like [`Simulation::step`], with the same results,
    /// and returns where the tick's wall time went by `clock` (plan M5 D136):
    /// for benchmarks; a simulation without traffic reports nothing.
    pub fn step_timed(&mut self, clock: &impl crate::WallClock) -> Result<crate::ops::PhaseTimes> {
        let next = self
            .time
            .checked_add(TICK)
            .with_context(|| format!("cannot advance simulation past {}", self.time))?;
        let mut times = crate::ops::PhaseTimes::default();
        if let Some(operations) = &mut self.operations {
            let mut marks = crate::ops::timing::ClockTimer::new(clock, self.ticks);
            operations
                .step_with(&self.scenario, &mut marks)
                .context("flight operations failed")?;
            times = marks.finish();
        }
        self.time = next;
        self.ticks += 1;
        Ok(times)
    }

    /// A hash of the simulation's state (plan M5 D137): the time, every
    /// flight as the views show it and the network statistics, so that two
    /// runs that differ anywhere a user could see differ here. FNV-1a over
    /// the values' debug text, which formats floats exactly.
    pub fn fingerprint(&self) -> u64 {
        use std::fmt::Write as _;
        let mut hash = Fnv::default();
        // Writing into the hash cannot fail.
        let _ = write!(hash, "{:?}|{}", self.time, self.ticks);
        for flight in self.flights() {
            let _ = write!(hash, "|{flight:?}");
        }
        if let Some(statistics) = self.statistics() {
            let _ = write!(hash, "|{:?}", statistics.network());
        }
        hash.0
    }

    /// The airport and duration of the longest controlled taxi now under
    /// way, up to its stand or holding position (plan M5 D144); `None` if
    /// nobody taxis under control.
    pub fn longest_taxi_under_way(&self) -> Option<(Id<Airport>, crate::SimDuration)> {
        self.operations.as_ref()?.longest_taxi_under_way()
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

/// The 64-bit FNV-1a hash of the text written to it.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl std::fmt::Write for Fnv {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        for byte in text.bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(())
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
        // One flight at a time gives the same views (plan M9 PR 2).
        let mut listed = vec![false; sim.airframe_count()];
        for view in sim.flights() {
            assert_eq!(sim.flight(view.airframe()), Some(view));
            listed[view.airframe().as_usize()] = true;
        }
        let parked = listed
            .iter()
            .position(|listed| !listed)
            .context("a parked one")?;
        assert_eq!(sim.flight(Id::from_usize(parked)?), None);
        assert_eq!(sim.flight(Id::from_usize(sim.airframe_count())?), None);
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
        // West flow: a segregated pair, an independent runway (plan M5
        // D113) and 18 West as a second group (plan M7 D158).
        assert_eq!((landing.len(), departing.len()), (3, 3), "{landing:?}");
        assert!(
            landing
                .iter()
                .chain(&departing)
                .all(|n| n.starts_with("25") || n == "18"),
            "{landing:?} {departing:?}"
        );
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

    /// The world with region E's ground, where aircraft taxi under control.
    fn with_ground() -> Result<Simulation> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?
                .with_ground(crate::test_data::ground()?)?;
        let config = SimConfig::new("2026-07-08T10:00:00Z".parse()?, RngSeed::new(9));
        Simulation::new(config, scenario)
    }

    /// A clock that advances a microsecond on every reading.
    struct SteppingClock(std::cell::Cell<u64>);

    impl crate::WallClock for SteppingClock {
        fn now(&self) -> crate::WallInstant {
            let micros = self.0.get();
            self.0.set(micros + 1);
            crate::WallInstant::from_elapsed(std::time::Duration::from_micros(micros))
        }
    }

    #[test]
    fn test_step_timed_gives_the_results_of_step() -> Result<()> {
        let mut plain = with_ground()?;
        let mut timed = with_ground()?;
        let clock = SteppingClock(std::cell::Cell::new(0));
        for _ in 0..120 {
            plain.step()?;
            timed.step_timed(&clock)?;
        }
        assert_eq!(plain, timed);
        Ok(())
    }

    /// The process's monotonic clock: timed tails take real time.
    struct RealClock(std::time::Instant);

    impl crate::WallClock for RealClock {
        fn now(&self) -> crate::WallInstant {
            crate::WallInstant::from_elapsed(self.0.elapsed())
        }
    }

    #[test]
    fn test_step_timed_splits_the_tick_by_part() -> Result<()> {
        let mut sim = with_ground()?;
        let clock = RealClock(std::time::Instant::now());
        // Warm start puts nobody on the taxiways: pushbacks and roll-outs
        // take a few minutes.
        sim.run_ticks(210)?;
        let mut times = crate::ops::PhaseTimes::default();
        for _ in 0..30 {
            times += sim.step_timed(&clock)?;
        }
        for (part, time) in [
            ("ground control", times.ground_control),
            ("taxiing", times.taxiing),
            ("flying", times.flying),
            ("parked", times.parked),
        ] {
            assert!(!time.is_zero(), "{part}: {times:?}");
        }
        Ok(())
    }

    #[test]
    fn test_longest_taxi_under_way_grows_while_taxiing() -> Result<()> {
        let mut sim = with_ground()?;
        assert_eq!(
            sim.longest_taxi_under_way(),
            None,
            "warm start: nobody taxis"
        );
        sim.run_ticks(300)?;
        let (airport, longest) = sim.longest_taxi_under_way().context("taxiing")?;
        assert!(sim.world().airport(airport).is_some());
        assert!(longest > crate::SimDuration::ZERO);
        assert!(longest <= crate::SimDuration::from_secs(300));
        assert!(
            sim.statistics()
                .is_some_and(|s| s.network().longest_taxi() > crate::SimDuration::ZERO)
        );
        Ok(())
    }

    #[test]
    fn test_fingerprint_follows_the_state() -> Result<()> {
        let mut a = germany(StartMode::Warm)?;
        let mut b = germany(StartMode::Warm)?;
        assert_eq!(a.fingerprint(), b.fingerprint());
        a.run_ticks(30)?;
        b.run_ticks(30)?;
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.step()?;
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_ne!(simulation()?.fingerprint(), a.fingerprint());
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
