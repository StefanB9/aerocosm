//! aerocosm entry point.

mod cli;
mod run;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{Cli, Command},
    run::SystemClock,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing().context("failed to initialise tracing")?;
    info!(version = env!("CARGO_PKG_VERSION"), "aerocosm starting");
    match cli.command {
        Command::Run(args) => {
            info!(seed = args.seed.value(), start = %args.start, duration = %args.duration, "run started");
            let summary = run::run(&args, &SystemClock::new()).context("run failed")?;
            info!(
                ticks = summary.ticks,
                end = %summary.end,
                wall_ms = summary.wall.as_millis(),
                ticks_per_second = summary.ticks_per_second(),
                "run finished"
            );
        }
    }
    Ok(())
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
