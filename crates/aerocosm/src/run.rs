//! The `run` command: a headless batch run as fast as possible.

use std::{
    io::Read,
    time::{Duration, Instant},
};

use aerocosm_core::{
    AircraftCatalog, Scenario, SimConfig, SimTime, Simulation, TICK, WallClock, WallInstant,
    airline::AirlineCatalog,
    ground::{GroundDefinition, StandAreasDefinition},
    ops::{AirportStats, StartMode},
    traffic::TrafficTargets,
    world::World,
};
use anyhow::{Context, Result};

use crate::{
    cli::RunArgs,
    report::{AirportReport, airport_reports},
};

/// Outcome of a batch run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunSummary {
    /// Airports in the simulated world.
    pub(crate) airports: usize,
    /// Runways in the simulated world.
    pub(crate) runways: usize,
    /// Ticks simulated.
    pub(crate) ticks: u64,
    /// Simulated time at the end of the run.
    pub(crate) end: SimTime,
    /// Wall-clock time the run took.
    pub(crate) wall: Duration,
    /// Flights completed during the run.
    pub(crate) flights_flown: u64,
    /// Most aircraft airborne at once.
    pub(crate) peak_airborne: usize,
    /// Movements per target airport against the targets; empty without
    /// traffic.
    pub(crate) movements: Vec<AirportReport>,
    /// Network punctuality and delays; `None` without traffic.
    pub(crate) network: Option<AirportStats>,
}

impl RunSummary {
    /// Simulation throughput; infinite if the run took no measurable time.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn ticks_per_second(&self) -> f64 {
        // Tick counts far below 2^53 convert exactly.
        self.ticks as f64 / self.wall.as_secs_f64()
    }
}

fn read(mut reader: impl Read, what: &str) -> Result<String> {
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .with_context(|| format!("cannot read {what} file"))?;
    Ok(text)
}

/// Reads and validates a world file.
pub(crate) fn load_world(reader: impl Read) -> Result<World> {
    World::from_json(&read(reader, "world")?)
}

/// Reads the traffic inputs and checks them against `world`.
pub(crate) fn load_scenario(
    world: World,
    aircraft: impl Read,
    airlines: impl Read,
    traffic: impl Read,
) -> Result<Scenario> {
    Scenario::with_traffic(
        world,
        AircraftCatalog::from_json(&read(aircraft, "aircraft")?).context("aircraft catalog")?,
        AirlineCatalog::from_json(&read(airlines, "airline")?).context("airline catalog")?,
        TrafficTargets::from_json(&read(traffic, "traffic")?).context("traffic targets")?,
    )
}

/// Adds airport ground layouts and stand areas (plan M4) to `scenario`.
pub(crate) fn add_ground(
    scenario: Scenario,
    ground: impl Read,
    areas: impl Read,
) -> Result<Scenario> {
    scenario
        .with_ground(
            GroundDefinition::from_json(&read(ground, "ground")?).context("ground layouts")?,
        )?
        .with_stand_areas(
            StandAreasDefinition::from_json(&read(areas, "stand areas")?).context("stand areas")?,
        )
}

/// Runs `scenario` from `args.start` for `args.duration`, timing it with
/// `clock`.
pub(crate) fn run(
    args: &RunArgs,
    scenario: Scenario,
    clock: &impl WallClock,
) -> Result<RunSummary> {
    let end = args
        .start
        .checked_add(args.duration)
        .with_context(|| format!("{} + {} is out of range", args.start, args.duration))?;
    let mode = if args.cold_start {
        StartMode::Cold
    } else {
        StartMode::Warm
    };
    let config = SimConfig::new(args.start, args.seed).with_start_mode(mode);
    let mut simulation = Simulation::new(config, scenario).context("cannot set up the run")?;
    let started = clock.now();
    let mut peak_airborne = simulation.airborne();
    while end.duration_since(simulation.time()) >= TICK {
        simulation
            .step()
            .with_context(|| format!("simulation failed at {}", simulation.time()))?;
        peak_airborne = peak_airborne.max(simulation.airborne());
    }
    let wall = clock.now().saturating_since(started);
    let movements = simulation
        .scenario()
        .traffic()
        .map_or_else(Vec::new, |traffic| {
            airport_reports(&simulation, traffic.targets(), args.start)
        });
    Ok(RunSummary {
        airports: simulation.world().airport_count(),
        runways: simulation.world().runway_count(),
        ticks: simulation.ticks(),
        end: simulation.time(),
        wall,
        flights_flown: simulation.flights_flown(),
        peak_airborne,
        network: simulation
            .statistics()
            .map(aerocosm_core::ops::Statistics::network),
        movements,
    })
}

/// The process's monotonic clock.
pub(crate) struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// A clock whose origin is now.
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl WallClock for SystemClock {
    fn now(&self) -> WallInstant {
        WallInstant::from_elapsed(self.origin.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, time::Duration};

    use aerocosm_core::{RngSeed, SimDuration, world::World};
    use anyhow::{Context, Result};

    use super::*;

    /// A clock that advances a fixed amount on every reading.
    struct SteppingClock {
        now: Cell<Duration>,
        step: Duration,
    }

    impl WallClock for SteppingClock {
        fn now(&self) -> WallInstant {
            let now = self.now.get();
            self.now.set(now + self.step);
            WallInstant::from_elapsed(now)
        }
    }

    fn clock() -> SteppingClock {
        SteppingClock {
            now: Cell::new(Duration::ZERO),
            step: Duration::from_millis(500),
        }
    }

    fn args(duration: SimDuration) -> Result<RunArgs> {
        Ok(RunArgs {
            seed: RngSeed::new(3),
            start: "2026-11-05T00:00:00Z".parse()?,
            duration,
            world: None,
            aircraft: "assets/aircraft/types.json".into(),
            airlines: "assets/airlines/airlines.json".into(),
            traffic: "assets/traffic/germany.json".into(),
            no_traffic: false,
            cold_start: false,
            ground: "assets/ground/germany.ground.json".into(),
            stand_areas: "assets/ground/stand-areas.json".into(),
            no_ground: false,
        })
    }

    fn germany() -> Result<Scenario> {
        load_scenario(
            load_world(include_str!("../../../assets/world/germany.world.json").as_bytes())?,
            include_str!("../../../assets/aircraft/types.json").as_bytes(),
            include_str!("../../../assets/airlines/airlines.json").as_bytes(),
            include_str!("../../../assets/traffic/germany.json").as_bytes(),
        )
    }

    const ONE_AIRPORT: &str = r#"{
        "format_version": 2,
        "airports": [{
            "ident": "DE-0062", "name": "Test Field",
            "location": {"lat": 51.0, "lon": 10.0}, "elevation": 500.0,
            "kind": "airfield", "country": "DE"
        }],
        "runways": []
    }"#;

    #[test]
    fn test_load_world_reads_json() -> Result<()> {
        let world = load_world(ONE_AIRPORT.as_bytes())?;
        assert_eq!(world.airport_count(), 1);
        Ok(())
    }

    #[test]
    fn test_load_world_rejects_garbage() {
        assert!(load_world(&b"not json"[..]).is_err());
    }

    #[test]
    fn test_run_simulates_full_day() -> Result<()> {
        let args = args(SimDuration::from_hours(24))?;
        let summary = run(&args, Scenario::empty(), &clock())?;
        assert_eq!(summary.ticks, 86_400);
        assert_eq!(summary.end, "2026-11-06T00:00:00Z".parse()?);
        assert_eq!(summary.wall, Duration::from_millis(500));
        let world = load_world(ONE_AIRPORT.as_bytes())?;
        let with_world = run(&args, Scenario::new(world), &clock())?;
        assert_eq!((with_world.airports, with_world.runways), (1, 0));
        assert_eq!((with_world.flights_flown, with_world.peak_airborne), (0, 0));
        assert_eq!(with_world.movements.len(), 0);
        Ok(())
    }

    #[test]
    fn test_run_stops_on_last_whole_tick() -> Result<()> {
        let summary = run(
            &args(SimDuration::from_millis(2_750))?,
            Scenario::empty(),
            &clock(),
        )?;
        assert_eq!(summary.ticks, 2);
        assert_eq!(summary.end, "2026-11-05T00:00:02Z".parse()?);
        Ok(())
    }

    #[test]
    fn test_run_rejects_end_beyond_supported_time() -> Result<()> {
        let args = RunArgs {
            start: SimTime::MAX,
            ..args(SimDuration::from_secs(1))?
        };
        assert!(run(&args, Scenario::empty(), &clock()).is_err());
        Ok(())
    }

    #[test]
    fn test_add_ground_loads_layouts_and_stand_areas() -> Result<()> {
        let scenario = add_ground(
            germany()?,
            include_str!("../../../assets/ground/germany.ground.json").as_bytes(),
            include_str!("../../../assets/ground/stand-areas.json").as_bytes(),
        )?;
        assert!(scenario.ground().airports().len() > 20);
        assert!(add_ground(germany()?, b"{}".as_slice(), b"{}".as_slice()).is_err());
        Ok(())
    }

    #[test]
    fn test_run_with_traffic_reports_flights_and_movements() -> Result<()> {
        let args = RunArgs {
            start: "2026-07-08T10:00:00Z".parse()?,
            ..args(SimDuration::from_minutes(30))?
        };
        let summary = run(&args, germany()?, &clock())?;
        assert!(summary.flights_flown > 50, "{}", summary.flights_flown);
        assert!(summary.peak_airborne > 300, "{}", summary.peak_airborne);
        assert!(summary.movements.len() > 20);
        let frankfurt = summary
            .movements
            .iter()
            .find(|report| report.airport.to_string() == "EDDF")
            .context("EDDF")?;
        assert!(frankfurt.actual > 0 && frankfurt.expected > 20.0);
        assert!(frankfurt.stats.busiest_hour() > 0);
        assert_eq!(frankfurt.capacity.map(|c| c.per_hour), Some(104));
        let network = summary.network.context("network statistics")?;
        assert!(network.departures() > 50, "{}", network.departures());
        Ok(())
    }

    #[test]
    fn test_run_cold_start_begins_with_nothing_airborne() -> Result<()> {
        let args = RunArgs {
            start: "2026-07-08T10:00:00Z".parse()?,
            cold_start: true,
            ..args(SimDuration::from_secs(10))?
        };
        let summary = run(&args, germany()?, &clock())?;
        assert_eq!(summary.peak_airborne, 0);
        Ok(())
    }

    #[test]
    fn test_load_scenario_rejects_bad_catalogs() {
        let result = load_scenario(World::empty(), &b"{}"[..], &b"{}"[..], &b"{}"[..]);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_summary_ticks_per_second() {
        let summary = RunSummary {
            airports: 0,
            runways: 0,
            ticks: 1000,
            end: SimTime::UNIX_EPOCH,
            wall: Duration::from_millis(250),
            flights_flown: 0,
            peak_airborne: 0,
            movements: Vec::new(),
            network: None,
        };
        assert!((summary.ticks_per_second() - 4000.0).abs() < 1e-9);
        let instant = RunSummary {
            wall: Duration::ZERO,
            ..summary.clone()
        };
        assert!(instant.ticks_per_second().is_infinite());
    }

    #[test]
    fn test_system_clock_is_monotonic() {
        let clock = SystemClock::new();
        let first = clock.now();
        let second = clock.now();
        assert!(second >= first);
    }
}
