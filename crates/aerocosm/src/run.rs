//! The `run` command: a headless batch run as fast as possible.

use std::{
    io::Read,
    time::{Duration, Instant},
};

use aerocosm_core::{
    Scenario, SimConfig, SimTime, Simulation, WallClock, WallInstant, world::World,
};
use anyhow::{Context, Result};

use crate::cli::RunArgs;

/// Outcome of a batch run.
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

impl RunSummary {
    /// Simulation throughput; infinite if the run took no measurable time.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn ticks_per_second(&self) -> f64 {
        // Tick counts far below 2^53 convert exactly.
        self.ticks as f64 / self.wall.as_secs_f64()
    }
}

/// Reads and validates a world file.
pub(crate) fn load_world(mut reader: impl Read) -> Result<World> {
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .context("cannot read world file")?;
    World::from_json(&text)
}

/// Runs `world` from `args.start` for `args.duration`, timing it with `clock`.
pub(crate) fn run(args: &RunArgs, world: World, clock: &impl WallClock) -> Result<RunSummary> {
    let end = args
        .start
        .checked_add(args.duration)
        .with_context(|| format!("{} + {} is out of range", args.start, args.duration))?;
    let mut simulation =
        Simulation::new(SimConfig::new(args.start, args.seed), Scenario::new(world))?;
    let started = clock.now();
    let ticks = simulation
        .run_until(end)
        .with_context(|| format!("simulation failed at {}", simulation.time()))?;
    Ok(RunSummary {
        airports: simulation.world().airport_count(),
        runways: simulation.world().runway_count(),
        ticks,
        end: simulation.time(),
        wall: clock.now().saturating_since(started),
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
    use anyhow::Result;

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
        })
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
        let summary = run(&args, World::empty(), &clock())?;
        assert_eq!(summary.ticks, 86_400);
        assert_eq!(summary.end, "2026-11-06T00:00:00Z".parse()?);
        assert_eq!(summary.wall, Duration::from_millis(500));
        let with_world = run(&args, load_world(ONE_AIRPORT.as_bytes())?, &clock())?;
        assert_eq!((with_world.airports, with_world.runways), (1, 0));
        Ok(())
    }

    #[test]
    fn test_run_stops_on_last_whole_tick() -> Result<()> {
        let summary = run(
            &args(SimDuration::from_millis(2_750))?,
            World::empty(),
            &clock(),
        )?;
        assert_eq!(summary.ticks, 2);
        assert_eq!(summary.end, "2026-11-05T00:00:02Z".parse()?);
        Ok(())
    }

    #[test]
    fn test_run_rejects_end_beyond_supported_time() {
        let args = RunArgs {
            seed: RngSeed::new(0),
            start: SimTime::MAX,
            duration: SimDuration::from_secs(1),
            world: None,
        };
        assert!(run(&args, World::empty(), &clock()).is_err());
    }

    #[test]
    fn test_run_summary_ticks_per_second() {
        let summary = RunSummary {
            airports: 0,
            runways: 0,
            ticks: 1000,
            end: SimTime::UNIX_EPOCH,
            wall: Duration::from_millis(250),
        };
        assert!((summary.ticks_per_second() - 4000.0).abs() < 1e-9);
        let instant = RunSummary {
            wall: Duration::ZERO,
            ..summary
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
