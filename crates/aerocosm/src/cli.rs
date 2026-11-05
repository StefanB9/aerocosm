//! Command-line interface.

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
        ])?;
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
