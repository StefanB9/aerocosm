//! The `run` command: a headless batch run as fast as possible.

use std::time::{Duration, Instant};

use aerocosm_core::{SimConfig, SimTime, Simulation, WallClock, WallInstant};
use anyhow::{Context, Result};

use crate::cli::RunArgs;

/// Outcome of a batch run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RunSummary {
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

/// Runs the simulation from `args.start` for `args.duration`, timing it with
/// `clock`.
pub(crate) fn run(args: &RunArgs, clock: &impl WallClock) -> Result<RunSummary> {
    let end = args
        .start
        .checked_add(args.duration)
        .with_context(|| format!("{} + {} is out of range", args.start, args.duration))?;
    let mut simulation = Simulation::new(SimConfig::new(args.start, args.seed));
    let started = clock.now();
    let ticks = simulation
        .run_until(end)
        .with_context(|| format!("simulation failed at {}", simulation.time()))?;
    Ok(RunSummary {
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

    use aerocosm_core::{RngSeed, SimDuration};
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
        })
    }

    #[test]
    fn test_run_simulates_full_day() -> Result<()> {
        let args = args(SimDuration::from_hours(24))?;
        let summary = run(&args, &clock())?;
        assert_eq!(summary.ticks, 86_400);
        assert_eq!(summary.end, "2026-11-06T00:00:00Z".parse()?);
        assert_eq!(summary.wall, Duration::from_millis(500));
        Ok(())
    }

    #[test]
    fn test_run_stops_on_last_whole_tick() -> Result<()> {
        let summary = run(&args(SimDuration::from_millis(2_750))?, &clock())?;
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
        };
        assert!(run(&args, &clock()).is_err());
    }

    #[test]
    fn test_run_summary_ticks_per_second() {
        let summary = RunSummary {
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
