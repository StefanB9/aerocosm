//! `aerocosm-data import`: regenerates the world and basemap assets from local
//! downloads.

mod cli;

use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use aerocosm_core::{
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use aerocosm_data::{
    import_all,
    natural_earth::NaturalEarthSources,
    osm::{OsmSource, StandSizes, ground_airports, import_ground, overpass_query},
    ourairports::OurAirportsSources,
};
use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command, GroundArgs, ImportArgs, QueriesArgs};

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
        Command::Ground(args) => ground(&args),
        Command::Queries(args) => queries(&args),
    }
}

/// One `IDENT<TAB>query` line per airport with a ground layout.
fn query_lines(world: &World, traffic: &TrafficTargets) -> Result<String> {
    let airports = ground_airports(world, traffic);
    let mut lines = String::with_capacity(airports.len() * 320);
    for airport in airports {
        lines.push_str(airport.as_str());
        lines.push('\t');
        lines.push_str(&overpass_query(world, airport)?);
        lines.push('\n');
    }
    Ok(lines)
}

fn queries(args: &QueriesArgs) -> Result<()> {
    let paths = GroundPaths::new(&args.assets);
    let read = |path: &Path| {
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
    };
    let world = World::from_json(&read(&paths.world)?).context("invalid world file")?;
    let traffic =
        TrafficTargets::from_json(&read(&paths.traffic)?).context("invalid traffic file")?;
    let lines = query_lines(&world, &traffic)?;
    fs::write(&args.out, &lines).with_context(|| format!("cannot write {}", args.out.display()))?;
    info!(path = %args.out.display(), airports = lines.lines().count(), "wrote queries");
    Ok(())
}

/// The asset files the ground import reads and writes.
struct GroundPaths {
    world: PathBuf,
    traffic: PathBuf,
    ground: PathBuf,
    sizes: PathBuf,
}

impl GroundPaths {
    fn new(assets: &Path) -> Self {
        Self {
            world: assets.join("world").join("germany.world.json"),
            traffic: assets.join("traffic").join("germany.json"),
            ground: assets.join("ground").join("germany.ground.json"),
            sizes: assets.join("ground").join("stand-sizes.json"),
        }
    }
}

/// The airport a download is for: its file name, `EDDF.json` → `EDDF`.
fn source_airport(path: &Path) -> Option<AirportIdent> {
    if path.extension()? != "json" {
        return None;
    }
    path.file_stem()?.to_str()?.parse().ok()
}

fn ground(args: &GroundArgs) -> Result<()> {
    let paths = GroundPaths::new(&args.assets);
    let read = |path: &Path| {
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
    };
    let world = World::from_json(&read(&paths.world)?).context("invalid world file")?;
    let traffic =
        TrafficTargets::from_json(&read(&paths.traffic)?).context("invalid traffic file")?;
    let mut sources = Vec::new();
    let entries =
        fs::read_dir(&args.osm).with_context(|| format!("cannot list {}", args.osm.display()))?;
    for entry in entries {
        let path = entry.context("cannot list the OSM directory")?.path();
        if let Some(airport) = source_airport(&path) {
            sources.push(OsmSource {
                airport,
                json: read(&path)?,
            });
        }
    }
    let sizes = StandSizes::from_json(&read(&paths.sizes)?).context("invalid stand size file")?;
    let output = import_ground(&sources, &world, &traffic, &sizes)?;
    for report in &output.reports {
        info!(%report, "airport");
    }
    if let Some(dir) = &args.rejected {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        for (airport, json) in &output.rejected {
            let path = dir.join(format!("{airport}.ground.json"));
            fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))?;
        }
    }
    if let Some(parent) = paths.ground.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&paths.ground, &output.ground_json)
        .with_context(|| format!("cannot write {}", paths.ground.display()))?;
    info!(path = %paths.ground.display(), bytes = output.ground_json.len(), "wrote asset");
    Ok(())
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
    fn test_ground_paths_and_source_names() {
        let paths = GroundPaths::new(Path::new("assets"));
        assert_eq!(paths.world, Path::new("assets/world/germany.world.json"));
        assert_eq!(paths.traffic, Path::new("assets/traffic/germany.json"));
        assert_eq!(paths.ground, Path::new("assets/ground/germany.ground.json"));
        assert_eq!(
            source_airport(Path::new("dl/EDDF.json")).map(|a| a.to_string()),
            Some("EDDF".to_owned())
        );
        assert_eq!(source_airport(Path::new("dl/EDDF.tmp")), None);
        assert_eq!(source_airport(Path::new("dl/notes.json")), None);
    }

    #[test]
    fn test_query_lines_are_tab_separated() -> Result<()> {
        let world = World::from_json(include_str!("../../../assets/world/germany.world.json"))?;
        let traffic =
            TrafficTargets::from_json(include_str!("../../../assets/traffic/germany.json"))?;
        let lines = query_lines(&world, &traffic)?;
        assert_eq!(lines.lines().count(), 24);
        let first = lines.lines().next().context("a line")?;
        let (ident, query) = first.split_once('\t').context("a tab")?;
        assert_eq!(ident.len(), 4);
        assert!(query.starts_with("[out:json]"));
        Ok(())
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
