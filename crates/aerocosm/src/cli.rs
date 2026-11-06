//! Command-line interface.

use std::path::PathBuf;

use aerocosm_core::{RngSeed, SimDuration, SimTime};
use clap::{Args, Parser, Subcommand, builder::TypedValueParser};

/// A complete aviation world in miniature.
#[derive(Debug, Parser)]
#[command(name = "aerocosm", version, about)]
pub(crate) struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Run the simulation headless, as fast as possible, and report a summary.
    Run(RunArgs),
}

/// Options for [`Command::Run`].
#[derive(Debug, Clone, Args)]
pub(crate) struct RunArgs {
    /// Root random seed; the same seed always reproduces the same run.
    #[arg(long, default_value = "0", value_parser = clap::value_parser!(u64).map(RngSeed::new))]
    pub(crate) seed: RngSeed,

    /// Simulated start time (UTC), e.g. 2026-11-05T06:00:00Z.
    #[arg(long, default_value = "2026-01-01T00:00:00Z")]
    pub(crate) start: SimTime,

    /// How much simulated time to run, e.g. 24h, 1h30m, 7d.
    #[arg(long, default_value = "24h")]
    pub(crate) duration: SimDuration,

    /// World file to simulate, e.g. assets/world/germany.world.json. Without it
    /// the world is empty.
    #[arg(long)]
    pub(crate) world: Option<PathBuf>,

    /// Aircraft catalog, used with --world unless --no-traffic.
    #[arg(long, default_value = "assets/aircraft/types.json")]
    pub(crate) aircraft: PathBuf,

    /// Airline catalog, used with --world unless --no-traffic.
    #[arg(long, default_value = "assets/airlines/airlines.json")]
    pub(crate) airlines: PathBuf,

    /// Traffic calibration targets, used with --world unless --no-traffic.
    #[arg(long, default_value = "assets/traffic/germany.json")]
    pub(crate) traffic: PathBuf,

    /// Simulate the static world only, without schedules and flights.
    #[arg(long)]
    pub(crate) no_traffic: bool,

    /// Start with every aircraft on the ground instead of mid-flight.
    #[arg(long)]
    pub(crate) cold_start: bool,
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, bail};

    use super::*;

    fn run_args(command_line: &[&str]) -> Result<RunArgs> {
        let cli = Cli::try_parse_from(command_line).context("arguments must parse")?;
        match cli.command {
            Command::Run(args) => Ok(args),
        }
    }

    #[test]
    fn test_cli_run_defaults() -> Result<()> {
        let args = run_args(&["aerocosm", "run"])?;
        assert_eq!(args.seed, RngSeed::new(0));
        assert_eq!(args.start, "2026-01-01T00:00:00Z".parse::<SimTime>()?);
        assert_eq!(args.duration, SimDuration::from_hours(24));
        assert_eq!(args.world, None);
        assert_eq!(
            args.aircraft,
            std::path::Path::new("assets/aircraft/types.json")
        );
        assert_eq!(
            args.airlines,
            std::path::Path::new("assets/airlines/airlines.json")
        );
        assert_eq!(
            args.traffic,
            std::path::Path::new("assets/traffic/germany.json")
        );
        assert!(!args.no_traffic && !args.cold_start);
        Ok(())
    }

    #[test]
    fn test_cli_run_traffic_flags() -> Result<()> {
        let args = run_args(&[
            "aerocosm",
            "run",
            "--cold-start",
            "--no-traffic",
            "--traffic",
            "other.json",
        ])?;
        assert!(args.cold_start && args.no_traffic);
        assert_eq!(args.traffic, std::path::Path::new("other.json"));
        Ok(())
    }

    #[test]
    fn test_cli_run_custom_values() -> Result<()> {
        let args = run_args(&[
            "aerocosm",
            "run",
            "--seed",
            "7",
            "--start",
            "2026-11-05T06:00:00Z",
            "--duration",
            "1h30m",
            "--world",
            "assets/world/germany.world.json",
        ])?;
        assert_eq!(
            args.world.as_deref(),
            Some(std::path::Path::new("assets/world/germany.world.json"))
        );
        assert_eq!(args.seed, RngSeed::new(7));
        assert_eq!(args.start, "2026-11-05T06:00:00Z".parse::<SimTime>()?);
        assert_eq!(args.duration, SimDuration::from_minutes(90));
        Ok(())
    }

    #[test]
    fn test_cli_rejects_malformed_values() -> Result<()> {
        for argv in [
            ["aerocosm", "run", "--duration", "soon"],
            ["aerocosm", "run", "--start", "tomorrow"],
            ["aerocosm", "run", "--seed", "-1"],
        ] {
            if Cli::try_parse_from(argv).is_ok() {
                bail!("{argv:?} must be rejected");
            }
        }
        Ok(())
    }

    #[test]
    fn test_cli_requires_subcommand() {
        assert!(Cli::try_parse_from(["aerocosm"]).is_err());
    }
}
