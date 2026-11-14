//! `aerocosm-data import`: regenerates the world and basemap assets from local
//! downloads.

mod cli;

use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use aerocosm_core::{
    ground::{ground_file, ground_region},
    traffic::TrafficTargets,
    units::Degrees,
    world::{World, WorldDefinition, codes::AirportIdent},
};
use aerocosm_data::{
    GlobeSources, import_all, import_globe,
    natural_earth::NaturalEarthSources,
    opensky::{Aggregator, read_openflights},
    osm::{OsmInput, OsmSource, ground_airports, import_ground, overpass_query, query_box},
    ourairports::{OurAirportsSources, read_closed_airports},
    wind::parse_wind,
    world_model,
};
use anyhow::{Context, Result};
use clap::Parser;
use flate2::read::GzDecoder;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::cli::{
    Cli, Command, GlobeArgs, GroundArgs, ImportArgs, ObservedArgs, QueriesArgs, WindRequestsArgs,
};

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
        Command::Globe(args) => globe(&args),
        Command::WindRequests(args) => wind_requests(&args),
        Command::Observed(args) => observed(&args),
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

/// Every airport's [`query_box`] as one `GeoJSON` multipolygon feature.
fn box_polygons(world: &World, traffic: &TrafficTargets) -> Result<String> {
    let mut polygons = Vec::new();
    for airport in ground_airports(world, traffic) {
        let [south, west, north, east] = query_box(world, airport)?;
        polygons.push(serde_json::json!([[
            [west, south],
            [east, south],
            [east, north],
            [west, north],
            [west, south]
        ]]));
    }
    let feature = serde_json::json!({
        "type": "Feature",
        "properties": {},
        "geometry": {"type": "MultiPolygon", "coordinates": polygons},
    });
    Ok(feature.to_string())
}

fn queries(args: &QueriesArgs) -> Result<()> {
    let paths = GroundPaths::new(&args.assets);
    let (world, traffic) = paths.model()?;
    let lines = query_lines(&world, &traffic)?;
    fs::write(&args.out, &lines).with_context(|| format!("cannot write {}", args.out.display()))?;
    info!(path = %args.out.display(), airports = lines.lines().count(), "wrote queries");
    if let Some(path) = &args.boxes {
        fs::write(path, box_polygons(&world, &traffic)?)
            .with_context(|| format!("cannot write {}", path.display()))?;
        info!(path = %path.display(), "wrote boxes");
    }
    Ok(())
}

/// The asset files the ground import reads and writes.
struct GroundPaths {
    world: PathBuf,
    aircraft: PathBuf,
    observed: PathBuf,
    /// The directory of the regional ground assets.
    ground: PathBuf,
}

impl GroundPaths {
    fn new(assets: &Path) -> Self {
        Self {
            world: assets.join("world").join("world.bin"),
            aircraft: assets.join("aircraft").join("types.json"),
            observed: assets.join("traffic").join("observed.bin"),
            ground: assets.join("ground"),
        }
    }

    /// The world and its traffic model.
    fn model(&self) -> Result<(World, TrafficTargets)> {
        let bytes =
            |path: &Path| fs::read(path).with_context(|| format!("cannot read {}", path.display()));
        let aircraft = fs::read_to_string(&self.aircraft)
            .with_context(|| format!("cannot read {}", self.aircraft.display()))?;
        world_model(&bytes(&self.world)?, &aircraft, &bytes(&self.observed)?)
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
    let (world, traffic) = paths.model()?;
    let mut sources = Vec::new();
    let (text, airports);
    let input = if let Some(opl) = &args.opl {
        text = read(opl)?;
        airports = ground_airports(&world, &traffic);
        OsmInput::Opl {
            text: &text,
            airports: &airports,
        }
    } else {
        let dir = args.osm.as_deref().context("--osm or --opl")?;
        let entries =
            fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))?;
        for entry in entries {
            let path = entry.context("cannot list the OSM directory")?.path();
            if let Some(airport) = source_airport(&path) {
                sources.push(OsmSource {
                    airport,
                    json: read(&path)?,
                });
            }
        }
        OsmInput::Overpass(&sources)
    };
    let output = import_ground(input, &world, &traffic)?;
    // The quality gate per region (plan M5 D118).
    let mut gate: BTreeMap<Option<char>, (usize, usize)> = BTreeMap::new();
    for report in &output.reports {
        let entry = gate.entry(ground_region(report.airport)).or_default();
        if report.rejected.is_none() {
            entry.0 += 1;
            info!(%report, "airport");
        } else {
            entry.1 += 1;
            tracing::debug!(%report, "airport");
        }
    }
    for (region, (passed, rejected)) in &gate {
        info!(region = %ground_file(*region), passed, rejected, "quality gate");
    }
    if let Some(dir) = &args.rejected {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        for (airport, json) in &output.rejected {
            let path = dir.join(format!("{airport}.ground.json"));
            fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))?;
        }
    }
    fs::create_dir_all(&paths.ground)
        .with_context(|| format!("cannot create {}", paths.ground.display()))?;
    for (region, bytes) in &output.regions {
        let path = paths.ground.join(ground_file(*region));
        fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
        info!(path = %path.display(), bytes = bytes.len(), "wrote asset");
    }
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

/// Imports the globe's world asset (plan M5).
fn globe(args: &GlobeArgs) -> Result<()> {
    let inputs = InputPaths::new(&args.ourairports, &args.natural_earth);
    let open = |path: &Path| -> Result<BufReader<File>> {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Ok(BufReader::new(file))
    };
    let winds = match &args.winds {
        Some(dir) => read_winds(dir)?,
        None => HashMap::new(),
    };
    let output = import_globe(
        GlobeSources {
            airports: open(&inputs.airports)?,
            runways: open(&inputs.runways)?,
            natural_earth: NaturalEarthSources {
                countries: open(&inputs.countries)?,
                borders: open(&inputs.borders)?,
                state_borders: open(&inputs.state_borders)?,
                lakes: open(&inputs.lakes)?,
                rivers: open(&inputs.rivers)?,
                places: open(&inputs.places)?,
            },
            zones: open(&args.zones)?,
        },
        &winds,
    )?;
    for (path, bytes) in [
        (
            args.out.join("world").join("world.bin"),
            output.world.to_asset()?,
        ),
        (
            args.out.join("basemap").join("world.bin"),
            output.basemap.to_asset()?,
        ),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        fs::write(&path, &bytes).with_context(|| format!("cannot write {}", path.display()))?;
        info!(path = %path.display(), bytes = bytes.len(), "wrote asset");
    }
    let report = &output.report;
    let scheduled = output.world.airports.iter().filter(|a| a.scheduled).count();
    info!(
        airports = report.airports,
        scheduled,
        surveyed_runways = report.surveyed_runways,
        approximate_runways = report.approximate_runways,
        zones_by_code = output.zones.by_code,
        zones_by_neighbour = output.zones.by_neighbour,
        zones_missing = output.zones.missing,
        winds = output.winds,
        "globe imported"
    );
    for (reason, count) in &report.skipped {
        info!(reason, count, "skipped records");
    }
    for (reason, count) in &report.fallbacks {
        info!(reason, count, "applied fallback");
    }
    Ok(())
}

/// The prevailing winds in a directory of POWER responses, `<IDENT>.json`.
fn read_winds(dir: &Path) -> Result<HashMap<AirportIdent, Degrees>> {
    let mut winds = HashMap::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let Some(ident) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<AirportIdent>().ok())
        else {
            continue;
        };
        let text =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        if let Some(wind) = parse_wind(&text).with_context(|| format!("in {}", path.display()))? {
            winds.insert(ident, wind);
        }
    }
    Ok(winds)
}

/// Writes the NASA POWER requests for the globe's scheduled airports.
fn wind_requests(args: &WindRequestsArgs) -> Result<()> {
    let bytes =
        fs::read(&args.world).with_context(|| format!("cannot read {}", args.world.display()))?;
    let world = WorldDefinition::from_asset(&bytes)?;
    let lines = aerocosm_data::wind::wind_requests(&world.airports);
    fs::write(&args.out, &lines).with_context(|| format!("cannot write {}", args.out.display()))?;
    info!(path = %args.out.display(), requests = lines.lines().count(), "wrote wind requests");
    Ok(())
}

/// Aggregates the `OpenSky` flight lists into the observed traffic asset.
fn observed(args: &ObservedArgs) -> Result<()> {
    let bytes =
        fs::read(&args.world).with_context(|| format!("cannot read {}", args.world.display()))?;
    let world = WorldDefinition::from_asset(&bytes)?;
    let airlines = read_openflights(BufReader::new(
        File::open(&args.airlines)
            .with_context(|| format!("cannot open {}", args.airlines.display()))?,
    ))?;
    let closed = read_closed_airports(BufReader::new(
        File::open(&args.airports)
            .with_context(|| format!("cannot open {}", args.airports.display()))?,
    ))?;
    let mut aggregator = Aggregator::new(&world, args.year).with_closed(&closed);
    for path in &args.flights {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        aggregator
            .add(BufReader::new(GzDecoder::new(BufReader::new(file))))
            .with_context(|| format!("in {}", path.display()))?;
        info!(path = %path.display(), "flight list counted");
    }
    let (observed, report) = aggregator.finish(&airlines, args.min_flights);
    let path = args.out.join("traffic").join("observed.bin");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let bytes = observed.to_asset()?;
    fs::write(&path, &bytes).with_context(|| format!("cannot write {}", path.display()))?;
    let flights: u64 = observed
        .airports
        .iter()
        .map(|a| u64::from(a.departures))
        .sum();
    info!(
        path = %path.display(),
        bytes = bytes.len(),
        read = report.flights,
        counted = flights,
        airports = observed.airports.len(),
        airlines = observed.airlines.len(),
        small_airlines = report.small_airlines,
        "wrote observed traffic"
    );
    for (reason, count) in &report.rejected {
        info!(reason, count, "rejected flights");
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
        assert_eq!(paths.world, Path::new("assets/world/world.bin"));
        assert_eq!(paths.observed, Path::new("assets/traffic/observed.bin"));
        assert_eq!(paths.ground, Path::new("assets/ground"));
        assert_eq!(
            source_airport(Path::new("dl/EDDF.json")).map(|a| a.to_string()),
            Some("EDDF".to_owned())
        );
        assert_eq!(source_airport(Path::new("dl/EDDF.tmp")), None);
        assert_eq!(source_airport(Path::new("dl/notes.json")), None);
    }

    #[test]
    fn test_box_polygons_hold_one_box_per_airport() -> Result<()> {
        let (world, traffic) = world_model(
            include_bytes!("../../../assets/world/world.bin"),
            include_str!("../../../assets/aircraft/types.json"),
            include_bytes!("../../../assets/traffic/observed.bin"),
        )?;
        let text = box_polygons(&world, &traffic)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        assert_eq!(value["geometry"]["type"], "MultiPolygon");
        let polygons = value["geometry"]["coordinates"]
            .as_array()
            .context("coordinates")?;
        assert_eq!(polygons.len(), traffic.airports().count());
        // Closed rings, west to east, south to north.
        let ring = polygons[0][0].as_array().context("a ring")?;
        assert_eq!(ring.len(), 5);
        assert_eq!(ring[0], ring[4]);
        let at = |i: usize, j: usize| ring[i][j].as_f64().context("a number");
        assert!(at(0, 0)? < at(1, 0)? && at(0, 1)? < at(2, 1)?);
        Ok(())
    }

    #[test]
    fn test_query_lines_are_tab_separated() -> Result<()> {
        let (world, traffic) = world_model(
            include_bytes!("../../../assets/world/world.bin"),
            include_str!("../../../assets/aircraft/types.json"),
            include_bytes!("../../../assets/traffic/observed.bin"),
        )?;
        let lines = query_lines(&world, &traffic)?;
        // Every scheduled airport (plan M5 D103).
        assert_eq!(lines.lines().count(), traffic.airports().count());
        for line in lines.lines() {
            let (ident, query) = line.split_once('\t').context("a tab")?;
            assert!(!ident.is_empty() && !ident.contains(' '), "{ident}");
            assert!(query.starts_with("[out:json]"), "{query}");
        }
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
