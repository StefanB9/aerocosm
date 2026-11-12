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
    let scenario = run::load_scenario(
        world,
        open(&args.aircraft)?,
        open(&args.airlines)?,
        open(&args.traffic)?,
    )
    .context("cannot load the traffic inputs")?;
    if args.no_ground {
        return Ok(scenario);
    }
    run::add_ground(scenario, open(&args.ground)?, open(&args.stand_areas)?)
        .context("cannot load the ground layouts")
}

/// Movements within this fraction of the target pass the check.
const MOVEMENT_TOLERANCE: f64 = 0.05;

/// Minutes of a duration, for logging.
fn minutes(duration: aerocosm_core::SimDuration) -> f64 {
    duration.as_secs_f64() / 60.0
}

/// Network punctuality, then one line per target airport: movements against
/// the target with punctuality and delays; airports off target, or busier in
/// an hour than their declared capacity, are warnings.
fn log_movements(summary: &run::RunSummary, days: f64) {
    if let Some(network) = &summary.network {
        info!(
            departures = network.departures(),
            on_time_departures_pct = network.on_time_departures() * 100.0,
            average_departure_delay_min = minutes(network.average_departure_delay()),
            arrivals = network.arrivals(),
            on_time_arrivals_pct = network.on_time_arrivals() * 100.0,
            average_arrival_delay_min = minutes(network.average_arrival_delay()),
            holds = network.holds(),
            holding_min = network.holding_minutes(),
            ground_delays = network.ground_delays(),
            ground_delay_min = network.ground_delay_minutes(),
            taxis = network.taxis(),
            taxi_delay_min = minutes(network.average_taxi_delay()),
            clearance_wait_min = minutes(network.average_clearance_wait()),
            stand_waits = network.stand_waits(),
            ground_conflicts = network.ground_conflicts(),
            reroutes = network.reroutes(),
            stand_shares = network.stand_shares(),
            "punctuality"
        );
    }
    for report in &summary.movements {
        let stats = &report.stats;
        let deviation_pct = report.deviation() * 100.0;
        let on_time_pct = stats.on_time_departures() * 100.0;
        let delay_min = minutes(stats.average_departure_delay());
        let holding_min = stats.holding_minutes();
        let ground_delays = stats.ground_delays();
        let busiest_hour = stats.busiest_hour();
        let taxi_delay_min = minutes(stats.average_taxi_delay());
        let clearance_wait_min = minutes(stats.average_clearance_wait());
        let stand_waits = stats.stand_waits();
        let ground_conflicts = stats.ground_conflicts();
        let reroutes = stats.reroutes();
        let stand_shares = stats.stand_shares();
        if report.within(MOVEMENT_TOLERANCE, days) {
            info!(airport = %report.airport, actual = report.actual, expected = report.expected.round(), deviation_pct, on_time_pct, delay_min, holding_min, ground_delays, busiest_hour, taxi_delay_min, clearance_wait_min, stand_waits, ground_conflicts, reroutes, stand_shares, "movements");
        } else {
            warn!(airport = %report.airport, actual = report.actual, expected = report.expected.round(), deviation_pct, on_time_pct, delay_min, holding_min, ground_delays, busiest_hour, taxi_delay_min, clearance_wait_min, stand_waits, ground_conflicts, reroutes, stand_shares, "movements off target");
        }
        if let Some((busiest, capacity)) = report.capacity_exceeded() {
            warn!(airport = %report.airport, busiest_hour = busiest, capacity, "capacity exceeded");
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
