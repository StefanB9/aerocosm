//! `aerocosm-data import`: regenerates the world and basemap assets from local
//! downloads.

mod cli;

use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use aerocosm_data::{
    import_all, natural_earth::NaturalEarthSources, ourairports::OurAirportsSources,
};
use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command, ImportArgs};

/// Where each input dataset file is expected.
struct InputPaths {
    airports: PathBuf,
    runways: PathBuf,
    countries: PathBuf,
    borders: PathBuf,
    state_borders: PathBuf,
    lakes: PathBuf,
    rivers: PathBuf,
    places: PathBuf,
}

impl InputPaths {
    fn new(ourairports: &Path, natural_earth: &Path) -> Self {
        let layer = |name: &str| natural_earth.join(format!("ne_10m_{name}.geojson"));
        Self {
            airports: ourairports.join("airports.csv"),
            runways: ourairports.join("runways.csv"),
            countries: layer("admin_0_countries"),
            borders: layer("admin_0_boundary_lines_land"),
            state_borders: layer("admin_1_states_provinces_lines"),
            lakes: layer("lakes"),
            rivers: layer("rivers_lake_centerlines"),
            places: layer("populated_places"),
        }
    }
}

/// Where the generated assets go.
struct OutputPaths {
    world: PathBuf,
    basemap: PathBuf,
}

impl OutputPaths {
    fn new(assets: &Path) -> Self {
        Self {
            world: assets.join("world").join("germany.world.json"),
            basemap: assets.join("basemap").join("germany.basemap.json"),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing().context("failed to initialise tracing")?;
    match cli.command {
        Command::Import(args) => import(&args),
    }
}

fn import(args: &ImportArgs) -> Result<()> {
    let inputs = InputPaths::new(&args.ourairports, &args.natural_earth);
    let open = |path: &Path| -> Result<BufReader<File>> {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Ok(BufReader::new(file))
    };
    let world = OurAirportsSources {
        airports: open(&inputs.airports)?,
        runways: open(&inputs.runways)?,
    };
    let basemap = NaturalEarthSources {
        countries: open(&inputs.countries)?,
        borders: open(&inputs.borders)?,
        state_borders: open(&inputs.state_borders)?,
        lakes: open(&inputs.lakes)?,
        rivers: open(&inputs.rivers)?,
        places: open(&inputs.places)?,
    };
    let output = import_all(world, basemap)?;

    let outputs = OutputPaths::new(&args.out);
    for (path, contents) in [
        (&outputs.world, &output.world_json),
        (&outputs.basemap, &output.basemap_json),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))?;
        info!(path = %path.display(), bytes = contents.len(), "wrote asset");
    }

    let report = &output.report;
    info!(
        airports = report.airports,
        surveyed_runways = report.surveyed_runways,
        approximate_runways = report.approximate_runways,
        "world imported"
    );
    for (reason, count) in &report.skipped {
        info!(reason, count, "skipped records");
    }
    for (reason, count) in &report.fallbacks {
        info!(reason, count, "applied fallback");
    }
    Ok(())
}

/// Logs to the console; `RUST_LOG` overrides the default filter.
fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,aerocosm_data=debug"))
        .context("invalid log filter")?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error))
        .context("a global subscriber is already set")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_input_paths_use_dataset_file_names() {
        let paths = InputPaths::new(Path::new("/data/oa"), Path::new("/data/ne"));
        assert_eq!(paths.airports, Path::new("/data/oa/airports.csv"));
        assert_eq!(paths.runways, Path::new("/data/oa/runways.csv"));
        assert_eq!(
            paths.countries,
            Path::new("/data/ne/ne_10m_admin_0_countries.geojson")
        );
        assert_eq!(
            paths.borders,
            Path::new("/data/ne/ne_10m_admin_0_boundary_lines_land.geojson")
        );
        assert_eq!(
            paths.state_borders,
            Path::new("/data/ne/ne_10m_admin_1_states_provinces_lines.geojson")
        );
        assert_eq!(paths.lakes, Path::new("/data/ne/ne_10m_lakes.geojson"));
        assert_eq!(
            paths.rivers,
            Path::new("/data/ne/ne_10m_rivers_lake_centerlines.geojson")
        );
        assert_eq!(
            paths.places,
            Path::new("/data/ne/ne_10m_populated_places.geojson")
        );
    }

    #[test]
    fn test_output_paths_under_assets() {
        let paths = OutputPaths::new(Path::new("assets"));
        assert_eq!(paths.world, Path::new("assets/world/germany.world.json"));
        assert_eq!(
            paths.basemap,
            Path::new("assets/basemap/germany.basemap.json")
        );
    }
}
