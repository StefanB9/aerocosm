//! aerocosm entry point.

use anyhow::{Context, Result};
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_tracing().context("failed to initialise tracing")?;
    info!(version = env!("CARGO_PKG_VERSION"), "aerocosm starting");
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
