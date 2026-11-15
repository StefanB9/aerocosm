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
    /// Run one benchmark scenario of the world and print its results, one
    /// `bench <name> <value> <unit>` line each (plan M5 D135).
    Bench(BenchArgs),
}

/// Options for [`Command::Bench`].
#[derive(Debug, Clone, Args)]
pub(crate) struct BenchArgs {
    /// The scenario: startup, tick-HH (e.g. tick-12) or day.
    pub(crate) scenario: crate::bench::world::ScenarioName,

    /// Root random seed.
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u64).map(RngSeed::new))]
    pub(crate) seed: RngSeed,

    /// Ticks run before measuring: warm start puts nobody on the taxiways,
    /// and half an hour fills them.
    #[arg(long, default_value = "1800")]
    pub(crate) settle: u32,

    /// Ticks measured one by one (default 600), or for `day` the ticks of
    /// the run (default a day).
    #[arg(long)]
    pub(crate) ticks: Option<u32>,

    /// Ticks timed by part after the measured ones.
    #[arg(long, default_value = "100")]
    pub(crate) split: u32,

    /// The world asset.
    #[arg(long, default_value = "assets/world/world.bin")]
    pub(crate) world: PathBuf,

    /// Aircraft catalog.
    #[arg(long, default_value = "assets/aircraft/types.json")]
    pub(crate) aircraft: PathBuf,

    /// Observed traffic.
    #[arg(long, default_value = "assets/traffic/observed.bin")]
    pub(crate) observed: PathBuf,

    /// Directory of the regional ground assets; every region is used.
    #[arg(long, default_value = "assets/ground")]
    pub(crate) ground: PathBuf,
}

impl BenchArgs {
    /// How long the scenario runs.
    pub(crate) fn lengths(&self) -> crate::bench::world::Lengths {
        use crate::bench::world::{DAY_TICKS, Lengths, ScenarioName};
        let default = if self.scenario == ScenarioName::Day {
            DAY_TICKS
        } else {
            600
        };
        Lengths {
            settle: self.settle,
            ticks: self.ticks.unwrap_or(default),
            split: self.split,
        }
    }
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

    /// World to simulate: the world asset, e.g. assets/world/world.bin, or a
    /// world file in JSON. Without it the world is empty.
    #[arg(long)]
    pub(crate) world: Option<PathBuf>,

    /// Aircraft catalog, used with --world unless --no-traffic.
    #[arg(long, default_value = "assets/aircraft/types.json")]
    pub(crate) aircraft: PathBuf,

    /// Observed traffic the world model is derived from (plan M5), used with
    /// --world unless --no-traffic.
    #[arg(long, default_value = "assets/traffic/observed.bin")]
    pub(crate) observed: PathBuf,

    /// Simulate the static world only, without schedules and flights.
    #[arg(long)]
    pub(crate) no_traffic: bool,

    /// Start with every aircraft on the ground instead of mid-flight.
    #[arg(long)]
    pub(crate) cold_start: bool,

    /// Directory of the regional airport ground assets, `*.ground.bin`
    /// (plan M4, M5 D117), used with traffic unless --no-ground.
    #[arg(long, default_value = "assets/ground")]
    pub(crate) ground: PathBuf,

    /// Taxi in straight lines everywhere, without ground layouts.
    #[arg(long)]
    pub(crate) no_ground: bool,
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, bail};

    use super::*;

    fn run_args(command_line: &[&str]) -> Result<RunArgs> {
        let cli = Cli::try_parse_from(command_line).context("arguments must parse")?;
        match cli.command {
            Command::Run(args) => Ok(args),
            Command::Bench(_) => bail!("expected run"),
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
            args.observed,
            std::path::Path::new("assets/traffic/observed.bin")
        );
        assert!(!args.no_traffic && !args.cold_start && !args.no_ground);
        assert_eq!(args.ground, std::path::Path::new("assets/ground"));
        assert!(run_args(&["aerocosm", "run", "--no-ground"])?.no_ground);
        Ok(())
    }

    #[test]
    fn test_cli_run_traffic_flags() -> Result<()> {
        let args = run_args(&[
            "aerocosm",
            "run",
            "--cold-start",
            "--no-traffic",
            "--observed",
            "other.bin",
        ])?;
        assert!(args.cold_start && args.no_traffic);
        assert_eq!(args.observed, std::path::Path::new("other.bin"));
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
            "assets/world/world.bin",
        ])?;
        assert_eq!(
            args.world.as_deref(),
            Some(std::path::Path::new("assets/world/world.bin"))
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
    fn test_cli_bench_defaults() -> Result<()> {
        use crate::bench::world::{DAY_TICKS, Lengths, ScenarioName};
        let bench = |line: &[&str]| -> Result<BenchArgs> {
            match Cli::try_parse_from(line)
                .context("arguments must parse")?
                .command
            {
                Command::Bench(args) => Ok(args),
                Command::Run(_) => bail!("expected bench"),
            }
        };
        let args = bench(&["aerocosm", "bench", "tick-12"])?;
        assert_eq!(args.scenario, ScenarioName::Tick(12));
        assert_eq!(args.seed, RngSeed::new(3));
        assert_eq!(
            args.lengths(),
            Lengths {
                settle: 1800,
                ticks: 600,
                split: 100
            }
        );
        assert_eq!(
            bench(&["aerocosm", "bench", "day"])?.lengths().ticks,
            DAY_TICKS
        );
        let args = bench(&["aerocosm", "bench", "day", "--ticks", "10"])?;
        assert_eq!(args.lengths().ticks, 10);
        assert!(Cli::try_parse_from(["aerocosm", "bench", "noon"]).is_err());
        Ok(())
    }

    #[test]
    fn test_cli_requires_subcommand() {
        assert!(Cli::try_parse_from(["aerocosm"]).is_err());
    }
}
