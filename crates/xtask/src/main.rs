//! Project tooling, run as `cargo xtask <task>` (alias in
//! `.cargo/config.toml`).
//!
//! All tooling is Rust: no shell, Python or other scripts in the project.

mod cli;
mod lock;
mod web;

use std::{env, path::Path};

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{Cli, Command, WebCommand},
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
    }
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
