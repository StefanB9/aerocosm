//! Command-line interface of the importer.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Regenerates aerocosm's data assets from public datasets.
#[derive(Debug, Parser)]
#[command(name = "aerocosm-data", version, about)]
pub(crate) struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Import Germany from local downloads (see docs/data-sources.md).
    Import(ImportArgs),
}

/// Options for [`Command::Import`].
#[derive(Debug, Args)]
pub(crate) struct ImportArgs {
    /// Directory containing `OurAirports` `airports.csv` and `runways.csv`.
    #[arg(long)]
    pub(crate) ourairports: PathBuf,
    /// Directory containing the Natural Earth 1:10m `GeoJSON` layers.
    #[arg(long)]
    pub(crate) natural_earth: PathBuf,
    /// Asset directory to write `world/` and `basemap/` into.
    #[arg(long, default_value = "assets")]
    pub(crate) out: PathBuf,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_cli_import_arguments() -> Result<()> {
        let cli = Cli::try_parse_from([
            "aerocosm-data",
            "import",
            "--ourairports",
            "dl/oa",
            "--natural-earth",
            "dl/ne",
            "--out",
            "assets",
        ])
        .context("must parse")?;
        let Command::Import(args) = cli.command;
        assert_eq!(args.ourairports, Path::new("dl/oa"));
        assert_eq!(args.natural_earth, Path::new("dl/ne"));
        assert_eq!(args.out, Path::new("assets"));
        Ok(())
    }

    #[test]
    fn test_cli_import_requires_inputs() {
        assert!(Cli::try_parse_from(["aerocosm-data", "import", "--out", "assets"]).is_err());
    }
}
