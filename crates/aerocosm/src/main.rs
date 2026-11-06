//! aerocosm entry point.

mod cli;
mod report;
mod run;

use std::{fs::File, io::BufReader, path::Path};

use aerocosm_core::Scenario;
use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{Cli, Command, RunArgs},
    run::SystemClock,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing().context("failed to initialise tracing")?;
    info!(version = env!("CARGO_PKG_VERSION"), "aerocosm starting");
    match cli.command {
        Command::Run(args) => {
            info!(seed = args.seed.value(), start = %args.start, duration = %args.duration, "run started");
            let scenario = load(&args)?;
            let summary = run::run(&args, scenario, &SystemClock::new()).context("run failed")?;
            info!(
                airports = summary.airports,
                runways = summary.runways,
                ticks = summary.ticks,
                end = %summary.end,
                wall_ms = summary.wall.as_millis(),
                ticks_per_second = summary.ticks_per_second(),
                flights_flown = summary.flights_flown,
                peak_airborne = summary.peak_airborne,
                "run finished"
            );
            log_movements(&summary, args.duration.as_secs_f64() / 86_400.0);
        }
    }
    Ok(())
}

/// Opens `path` for reading.
fn open(path: &Path) -> Result<BufReader<File>> {
    File::open(path)
        .map(BufReader::new)
        .with_context(|| format!("cannot open {}", path.display()))
}

/// The scenario the arguments describe: empty, a static world, or a world
/// with traffic.
fn load(args: &RunArgs) -> Result<Scenario> {
    let Some(path) = &args.world else {
        return Ok(Scenario::empty());
    };
    let world =
        run::load_world(open(path)?).with_context(|| format!("cannot load {}", path.display()))?;
    if args.no_traffic {
        return Ok(Scenario::new(world));
    }
    run::load_scenario(
        world,
        open(&args.aircraft)?,
        open(&args.airlines)?,
        open(&args.traffic)?,
    )
    .context("cannot load the traffic inputs")
}

/// Movements within this fraction of the target pass the check.
const MOVEMENT_TOLERANCE: f64 = 0.05;

/// One line per target airport; airports off target are warnings.
fn log_movements(summary: &run::RunSummary, days: f64) {
    for report in &summary.movements {
        let deviation_pct = report.deviation() * 100.0;
        if report.within(MOVEMENT_TOLERANCE, days) {
            info!(airport = %report.airport, actual = report.actual, expected = report.expected.round(), deviation_pct, "movements");
        } else {
            warn!(airport = %report.airport, actual = report.actual, expected = report.expected.round(), deviation_pct, "movements off target");
        }
    }
}

/// Logs to the console; `RUST_LOG` overrides the default filter.
fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,aerocosm=debug,aerocosm_core=debug"))
        .context("invalid log filter")?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error))
        .context("a global subscriber is already set")
}
