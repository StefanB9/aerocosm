//! Project tooling, run as `cargo xtask <task>` (alias in
//! `.cargo/config.toml`).
//!
//! All tooling is Rust: no shell, Python or other scripts in the project.

mod bench;
mod cli;
mod host;
mod lock;
mod web;

use std::{env, path::Path};

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{BenchArgs, BenchCommand, BenchGroup, Cli, Command, WebCommand},
    web::{
        build::{BuildOptions, Profile, SystemHost, build},
        serve::{Directory, serve},
    },
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing().context("failed to initialise tracing")?;
    // Tasks use paths relative to the workspace root, wherever cargo was run
    // from.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("xtask is not inside the workspace")?;
    env::set_current_dir(root).with_context(|| format!("cannot enter {}", root.display()))?;

    match cli.command {
        Command::Web(WebCommand::Build(args)) => build(
            &mut SystemHost,
            &BuildOptions {
                out: args.out,
                profile: if args.dev {
                    Profile::Dev
                } else {
                    Profile::Release
                },
                // Cargo sets CARGO for the programs it runs: use the same
                // toolchain.
                cargo: env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()),
            },
        ),
        Command::Web(WebCommand::Serve(args)) => serve(args.port, &Directory(args.dir)),
        Command::Bench(args) => run_bench(&args),
    }
}

/// Runs the benchmark suite or reports on its history, printing the
/// comparison with the previous record on this machine.
fn run_bench(args: &BenchArgs) -> Result<()> {
    use std::io::Write as _;
    let mut host = SystemHost;
    let text = if let Some(BenchCommand::Report) = args.command {
        let history = bench::history(&mut host, Path::new(bench::HISTORY))?;
        let latest = history.last().context("no benchmark records yet")?;
        bench::report::compare(latest, bench::report::previous(&history, latest))
    } else {
        let options = bench::Options {
            groups: args
                .groups
                .iter()
                .map(|group| match group {
                    BenchGroup::Startup => bench::Group::Startup,
                    BenchGroup::World => bench::Group::World,
                    BenchGroup::Micro => bench::Group::Micro,
                    BenchGroup::Viewer => bench::Group::Viewer,
                    BenchGroup::Window => bench::Group::Window,
                })
                .collect(),
            repeat: args.repeat,
            quick: args.quick,
            cargo: env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            os: env::consts::OS.to_owned(),
        };
        let outcome = bench::run(&mut host, &options)?;
        if outcome.recorded {
            info!(
                file = bench::HISTORY,
                "recorded; commit it with the change it measures"
            );
        } else {
            info!("not recorded: the working tree has changes, or the run was quick");
        }
        bench::compare_with_history(&mut host, &outcome.record)?
    };
    std::io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .context("cannot print the report")
}

/// Logs to the console; `RUST_LOG` overrides the default filter.
fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .context("invalid log filter")?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .try_init()
        .map_err(|error| anyhow::anyhow!(error))
        .context("a global subscriber is already set")
}
