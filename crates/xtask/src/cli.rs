//! Command-line interface of `cargo xtask`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Project tooling for aerocosm.
#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Task groups.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// The browser viewer.
    #[command(subcommand)]
    Web(WebCommand),
    /// The benchmark suite (plan M5 D141): run it, or report on its history.
    Bench(BenchArgs),
}

/// Options for `bench`: with no subcommand, run the suite.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct BenchArgs {
    #[command(subcommand)]
    pub(crate) command: Option<BenchCommand>,
    /// The groups to run (all by default).
    #[arg(value_enum)]
    pub(crate) groups: Vec<BenchGroup>,
    /// Runs of each start-up and tick scenario; results are their median.
    #[arg(long, default_value_t = 3)]
    pub(crate) repeat: usize,
    /// A few ticks and frames only, to check that everything runs; never
    /// recorded.
    #[arg(long)]
    pub(crate) quick: bool,
}

/// `cargo xtask bench …`
#[derive(Debug, Subcommand)]
pub(crate) enum BenchCommand {
    /// Compare the latest record with the previous one on the same machine.
    Report,
}

/// A group of benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum BenchGroup {
    /// Decoding and setting up the world.
    Startup,
    /// Ticks at four hours of the day and a simulated day.
    World,
    /// Criterion benchmarks of the core and the viewer.
    Micro,
    /// The viewer's frames, headless.
    Viewer,
    /// The viewer's frames in a real window (needs a display).
    Window,
}

/// `cargo xtask web …`
#[derive(Debug, Subcommand)]
pub(crate) enum WebCommand {
    /// Build the WebGPU and WebGL2 variants and the loader page.
    Build(BuildArgs),
    /// Serve a browser build on localhost.
    Serve(ServeArgs),
}

/// Options for `web build`.
#[derive(Debug, Args)]
pub(crate) struct BuildArgs {
    /// Output directory, relative to the workspace root.
    #[arg(long, default_value = "target/web")]
    pub(crate) out: PathBuf,
    /// Use the dev profile: builds faster, much larger and slower output.
    #[arg(long)]
    pub(crate) dev: bool,
}

/// Options for `web serve`.
#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    /// Directory to serve, relative to the workspace root.
    #[arg(long, default_value = "target/web")]
    pub(crate) dir: PathBuf,
    /// Port on 127.0.0.1.
    #[arg(long, default_value_t = 8080)]
    pub(crate) port: u16,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result, bail};

    use super::*;

    #[test]
    fn test_cli_web_build_defaults_and_flags() -> Result<()> {
        let Command::Web(WebCommand::Build(args)) = Cli::try_parse_from(["xtask", "web", "build"])
            .context("parse")?
            .command
        else {
            bail!("expected web build");
        };
        assert_eq!(args.out, Path::new("target/web"));
        assert!(!args.dev);
        let Command::Web(WebCommand::Build(args)) =
            Cli::try_parse_from(["xtask", "web", "build", "--dev", "--out", "x"])
                .context("parse")?
                .command
        else {
            bail!("expected web build");
        };
        assert!(args.dev);
        assert_eq!(args.out, Path::new("x"));
        Ok(())
    }

    #[test]
    fn test_cli_web_serve_defaults_and_port() -> Result<()> {
        let Command::Web(WebCommand::Serve(args)) =
            Cli::try_parse_from(["xtask", "web", "serve", "--port", "9000"])
                .context("parse")?
                .command
        else {
            bail!("expected web serve");
        };
        assert_eq!(args.port, 9000);
        assert_eq!(args.dir, Path::new("target/web"));
        Ok(())
    }

    #[test]
    fn test_cli_bench_groups_and_report() -> Result<()> {
        let Command::Bench(args) = Cli::try_parse_from(["xtask", "bench"])
            .context("parse")?
            .command
        else {
            bail!("expected bench");
        };
        assert!(args.groups.is_empty() && args.command.is_none() && !args.quick);
        assert_eq!(args.repeat, 3);
        let Command::Bench(args) = Cli::try_parse_from([
            "xtask", "bench", "world", "micro", "window", "--quick", "--repeat", "1",
        ])
        .context("parse")?
        .command
        else {
            bail!("expected bench");
        };
        assert_eq!(
            args.groups,
            [BenchGroup::World, BenchGroup::Micro, BenchGroup::Window]
        );
        assert!(args.quick);
        assert_eq!(args.repeat, 1);
        let Command::Bench(args) = Cli::try_parse_from(["xtask", "bench", "report"])
            .context("parse")?
            .command
        else {
            bail!("expected bench");
        };
        assert!(matches!(args.command, Some(BenchCommand::Report)));
        assert!(Cli::try_parse_from(["xtask", "bench", "graphics"]).is_err());
        Ok(())
    }

    #[test]
    fn test_cli_rejects_unknown_tasks() {
        assert!(Cli::try_parse_from(["xtask", "web", "deploy"]).is_err());
        assert!(Cli::try_parse_from(["xtask"]).is_err());
    }
}
