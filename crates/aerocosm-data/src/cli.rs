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
    /// Import airport ground layouts from Overpass downloads (plan M4).
    Ground(GroundArgs),
    /// Write the Overpass query of every airport with a ground layout.
    Queries(QueriesArgs),
    /// Import every aerodrome on the globe (plan M5).
    Globe(GlobeArgs),
    /// Write the NASA POWER wind request of every scheduled airport.
    WindRequests(WindRequestsArgs),
    /// Aggregate `OpenSky` flight lists into observed traffic (plan M5).
    Observed(ObservedArgs),
}

/// Options for [`Command::Observed`].
#[derive(Debug, Args)]
pub(crate) struct ObservedArgs {
    /// `OpenSky` flight lists, `.csv.gz` (one per month).
    #[arg(required = true)]
    pub(crate) flights: Vec<PathBuf>,
    /// `OpenFlights` `airlines.dat`.
    #[arg(long)]
    pub(crate) airlines: PathBuf,
    /// `OurAirports` `airports.csv`: where airports closed since were.
    #[arg(long)]
    pub(crate) airports: PathBuf,
    /// The globe's world asset.
    #[arg(long, default_value = "assets/world/world.bin")]
    pub(crate) world: PathBuf,
    /// The reference year.
    #[arg(long, default_value_t = 2019)]
    pub(crate) year: u16,
    /// Airlines need at least this many departures in the year.
    #[arg(long, default_value_t = 365)]
    pub(crate) min_flights: u32,
    /// Asset directory to write `traffic/observed.bin` into.
    #[arg(long, default_value = "assets")]
    pub(crate) out: PathBuf,
}

/// Options for [`Command::Globe`].
#[derive(Debug, Args)]
pub(crate) struct GlobeArgs {
    /// Directory containing `OurAirports` `airports.csv` and `runways.csv`.
    #[arg(long)]
    pub(crate) ourairports: PathBuf,
    /// Directory containing the Natural Earth 1:10m `GeoJSON` layers.
    #[arg(long)]
    pub(crate) natural_earth: PathBuf,
    /// The `airportsdata` `airports.csv`, for time zones.
    #[arg(long)]
    pub(crate) zones: PathBuf,
    /// Directory of downloaded NASA POWER responses, `<IDENT>.json`.
    #[arg(long)]
    pub(crate) winds: Option<PathBuf>,
    /// Asset directory to write `world/world.bin` into.
    #[arg(long, default_value = "assets")]
    pub(crate) out: PathBuf,
}

/// Options for [`Command::WindRequests`].
#[derive(Debug, Args)]
pub(crate) struct WindRequestsArgs {
    /// The globe's world asset.
    #[arg(long, default_value = "assets/world/world.bin")]
    pub(crate) world: PathBuf,
    /// File to write, one `IDENT<TAB>URL` line per scheduled airport.
    #[arg(long)]
    pub(crate) out: PathBuf,
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

/// Options for [`Command::Ground`].
#[derive(Debug, Args)]
pub(crate) struct GroundArgs {
    /// Directory containing one Overpass JSON file per airport, `<IDENT>.json`.
    #[arg(long)]
    pub(crate) osm: PathBuf,
    /// Asset directory: reads `world/` and `traffic/`, writes `ground/`.
    #[arg(long, default_value = "assets")]
    pub(crate) assets: PathBuf,
    /// Also write each rejected layout to `<dir>/<IDENT>.ground.json`.
    #[arg(long)]
    pub(crate) rejected: Option<PathBuf>,
}

/// Options for [`Command::Queries`].
#[derive(Debug, Args)]
pub(crate) struct QueriesArgs {
    /// Asset directory to read `world/` and `traffic/` from.
    #[arg(long, default_value = "assets")]
    pub(crate) assets: PathBuf,
    /// File to write, one `IDENT<TAB>query` line per airport.
    #[arg(long)]
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
        let Command::Import(args) = cli.command else {
            anyhow::bail!("expected the import command");
        };
        assert_eq!(args.ourairports, Path::new("dl/oa"));
        assert_eq!(args.natural_earth, Path::new("dl/ne"));
        assert_eq!(args.out, Path::new("assets"));
        Ok(())
    }

    #[test]
    fn test_cli_ground_arguments() -> Result<()> {
        let cli = Cli::try_parse_from(["aerocosm-data", "ground", "--osm", "dl/osm"])
            .context("must parse")?;
        let Command::Ground(args) = cli.command else {
            anyhow::bail!("expected the ground command");
        };
        assert_eq!(args.osm, Path::new("dl/osm"));
        assert_eq!(args.assets, Path::new("assets"));
        assert_eq!(args.rejected, None);
        assert!(Cli::try_parse_from(["aerocosm-data", "ground"]).is_err());
        Ok(())
    }

    #[test]
    fn test_cli_queries_arguments() -> Result<()> {
        let cli = Cli::try_parse_from(["aerocosm-data", "queries", "--out", "q.tsv"])
            .context("must parse")?;
        let Command::Queries(args) = cli.command else {
            anyhow::bail!("expected the queries command");
        };
        assert_eq!(args.out, Path::new("q.tsv"));
        assert_eq!(args.assets, Path::new("assets"));
        Ok(())
    }

    #[test]
    fn test_cli_import_requires_inputs() {
        assert!(Cli::try_parse_from(["aerocosm-data", "import", "--out", "assets"]).is_err());
    }

    #[test]
    fn test_cli_globe_arguments() -> Result<()> {
        let cli = Cli::try_parse_from([
            "aerocosm-data",
            "globe",
            "--ourairports",
            "dl/oa",
            "--natural-earth",
            "dl/ne",
            "--zones",
            "dl/airportsdata.csv",
        ])?;
        let Command::Globe(args) = cli.command else {
            anyhow::bail!("expected globe");
        };
        assert_eq!(args.zones, Path::new("dl/airportsdata.csv"));
        assert_eq!(args.winds, None);
        assert_eq!(args.out, Path::new("assets"));
        let cli = Cli::try_parse_from(["aerocosm-data", "wind-requests", "--out", "w.tsv"])?;
        let Command::WindRequests(args) = cli.command else {
            anyhow::bail!("expected wind-requests");
        };
        assert_eq!(args.world, Path::new("assets/world/world.bin"));
        Ok(())
    }

    #[test]
    fn test_cli_observed_arguments() -> Result<()> {
        let cli = Cli::try_parse_from([
            "aerocosm-data",
            "observed",
            "--airlines",
            "dl/airlines.dat",
            "--airports",
            "dl/airports.csv",
            "dl/flightlist_20190101_20190131.csv.gz",
            "dl/flightlist_20190201_20190228.csv.gz",
        ])?;
        let Command::Observed(args) = cli.command else {
            anyhow::bail!("expected observed");
        };
        assert_eq!(args.flights.len(), 2);
        assert_eq!(args.year, 2019);
        assert_eq!(args.min_flights, 365);
        assert_eq!(args.world, Path::new("assets/world/world.bin"));
        Ok(())
    }
}
