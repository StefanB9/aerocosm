//! Offline data pipeline: turns public datasets into aerocosm's world and
//! basemap files.

mod geometry;
pub mod natural_earth;
pub mod opensky;
pub mod osm;
pub mod ourairports;
pub mod wind;
pub mod zones;

use std::{
    collections::HashMap,
    hash::BuildHasher,
    io::{Cursor, Read},
};

use aerocosm_core::{
    units::Degrees,
    world::{WorldDefinition, codes::AirportIdent},
};
use anyhow::{Context, Result};

use crate::{
    natural_earth::{BasemapSettings, NaturalEarthSources, import_basemap, read_places},
    ourairports::{ImportReport, OurAirportsSources, WorldSettings, import_world},
    wind::assign_winds,
    zones::{ZoneReport, assign_zones, read_zones},
};

/// Reads a whole layer into memory.
fn buffer(mut reader: impl Read, layer: &str) -> Result<Cursor<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read Natural Earth {layer} layer"))?;
    Ok(Cursor::new(bytes))
}

/// The rendered asset files and the world import statistics.
#[derive(Debug, Clone)]
pub struct ImportOutput {
    /// Contents of the world file.
    pub world_json: String,
    /// Contents of the basemap file.
    pub basemap_json: String,
    /// What the world import did.
    pub report: ImportReport,
}

/// The inputs of the globe's world (plan M5 PR 1).
#[derive(Debug)]
pub struct GlobeSources<R> {
    /// `OurAirports` `airports.csv`.
    pub airports: R,
    /// `OurAirports` `runways.csv`.
    pub runways: R,
    /// Natural Earth layers: the basemap, and places for catchments.
    pub natural_earth: NaturalEarthSources<R>,
    /// The `airportsdata` table, for time zones.
    pub zones: R,
}

/// The globe's world and how it was made.
#[derive(Debug, Clone)]
pub struct GlobeOutput {
    /// The world.
    pub world: WorldDefinition,
    /// The world's basemap.
    pub basemap: aerocosm_basemap::Basemap,
    /// What the airport and runway import did.
    pub report: ImportReport,
    /// How the time zones were found.
    pub zones: ZoneReport,
    /// Airports given a prevailing wind.
    pub winds: usize,
}

/// Imports every aerodrome on the globe with time zones and the prevailing
/// winds downloaded so far. Pure: callers own all file I/O.
pub fn import_globe<R: Read, S: BuildHasher>(
    sources: GlobeSources<R>,
    winds: &HashMap<AirportIdent, Degrees, S>,
) -> Result<GlobeOutput> {
    let layers = sources.natural_earth;
    let natural_earth = NaturalEarthSources {
        countries: buffer(layers.countries, "countries")?,
        borders: buffer(layers.borders, "borders")?,
        state_borders: buffer(layers.state_borders, "state borders")?,
        lakes: buffer(layers.lakes, "lakes")?,
        rivers: buffer(layers.rivers, "rivers")?,
        places: buffer(layers.places, "places")?,
    };
    let places = read_places(natural_earth.places.get_ref().as_slice())
        .context("cannot read Natural Earth places")?;
    let basemap = import_basemap(natural_earth, &BasemapSettings::globe()?)
        .context("Natural Earth import failed")?;
    let import = import_world(
        OurAirportsSources {
            airports: sources.airports,
            runways: sources.runways,
        },
        &WorldSettings::globe()?,
        &places,
    )
    .context("OurAirports import failed")?;
    let mut world = import.definition;
    let zones = assign_zones(
        &mut world.airports,
        &read_zones(sources.zones).context("cannot read the time zone table")?,
    );
    let winds = assign_winds(&mut world.airports, winds);
    Ok(GlobeOutput {
        world,
        basemap,
        report: import.report,
        zones,
        winds,
    })
}

/// Imports Germany from both datasets and renders the asset files. Pure:
/// callers own all file I/O.
pub fn import_all<R: Read>(
    world: OurAirportsSources<R>,
    basemap: NaturalEarthSources<R>,
) -> Result<ImportOutput> {
    // The places layer feeds both the basemap and the airport catchments, so
    // the layers are buffered in memory (tens of megabytes, fine for an offline
    // tool) and read twice.
    let basemap = NaturalEarthSources {
        countries: buffer(basemap.countries, "countries")?,
        borders: buffer(basemap.borders, "borders")?,
        state_borders: buffer(basemap.state_borders, "state borders")?,
        lakes: buffer(basemap.lakes, "lakes")?,
        rivers: buffer(basemap.rivers, "rivers")?,
        places: buffer(basemap.places, "places")?,
    };
    let places = read_places(basemap.places.get_ref().as_slice())
        .context("cannot read Natural Earth places")?;
    let world = import_world(world, &WorldSettings::germany()?, &places)
        .context("OurAirports import failed")?;
    let basemap = import_basemap(basemap, &BasemapSettings::germany()?)
        .context("Natural Earth import failed")?;
    Ok(ImportOutput {
        world_json: world.definition.to_json_pretty()?,
        basemap_json: basemap.to_json()?,
        report: world.report,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use anyhow::{Context, Result};

    use super::*;

    fn empty_layer() -> Cursor<Vec<u8>> {
        Cursor::new(br#"{"type": "FeatureCollection", "features": []}"#.to_vec())
    }

    #[test]
    fn test_import_all_renders_both_files() -> Result<()> {
        let world = ourairports::OurAirportsSources {
            airports: Cursor::new(b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,scheduled_service,icao_code,iata_code\n1,EDFE,small_airport,Egelsbach,49.96,8.65,384,DE,yes,EDFE,\n".to_vec()),
            runways: Cursor::new(b"airport_ident,length_ft,width_ft,surface,closed,le_ident,le_latitude_deg,le_longitude_deg,le_heading_degT,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_heading_degT,he_displaced_threshold_ft\nEDFE,2756,66,ASP,0,09,,,,,27,,,,\n".to_vec()),
        };
        let basemap = natural_earth::NaturalEarthSources {
            countries: empty_layer(),
            borders: empty_layer(),
            state_borders: empty_layer(),
            lakes: empty_layer(),
            rivers: empty_layer(),
            places: empty_layer(),
        };
        let output = import_all(world, basemap)?;
        assert!(
            output.world_json.contains("\"catchment\""),
            "regional airports get a catchment"
        );
        assert!(aerocosm_core::world::World::from_json(&output.world_json).is_ok());
        assert!(aerocosm_basemap::Basemap::from_json(&output.basemap_json).is_ok());
        assert_eq!(output.report.airports, 1);
        Ok(())
    }

    #[test]
    fn test_import_globe_fills_zones_and_winds() -> Result<()> {
        let sources = GlobeSources {
            airports: Cursor::new(b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,scheduled_service,icao_code,iata_code\n1,EDDF,large_airport,Frankfurt,50.03,8.57,364,DE,yes,EDDF,FRA\n2,KJFK,large_airport,Kennedy,40.64,-73.78,13,US,yes,KJFK,JFK\n".to_vec()),
            runways: Cursor::new(b"airport_ident,length_ft,width_ft,surface,closed,le_ident,le_latitude_deg,le_longitude_deg,le_heading_degT,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_heading_degT,he_displaced_threshold_ft\nEDDF,13123,197,CON,0,07C,,,69.6,,25C,,,249.6,\nKJFK,14511,200,ASP,0,04L,,,31,,22R,,,211,\n".to_vec()),
            natural_earth: natural_earth::NaturalEarthSources {
                countries: empty_layer(),
                borders: empty_layer(),
                state_borders: empty_layer(),
                lakes: empty_layer(),
                rivers: empty_layer(),
                places: empty_layer(),
            },
            zones: Cursor::new(b"\"icao\",\"iata\",\"name\",\"city\",\"subd\",\"country\",\"elevation\",\"lat\",\"lon\",\"tz\",\"lid\"\n\"EDDF\",\"FRA\",\"F\",\"\",\"\",\"DE\",364,50.03,8.57,\"Europe/Berlin\",\"\"\n".to_vec()),
        };
        let mut winds = std::collections::HashMap::new();
        winds.insert("EDDF".parse()?, aerocosm_core::units::Degrees::new(232.3)?);
        let output = import_globe(sources, &winds)?;
        let airport = |ident: &str| {
            output
                .world
                .airports
                .iter()
                .find(|a| a.ident.as_str() == ident)
                .cloned()
        };
        let frankfurt = airport("EDDF").context("Frankfurt")?;
        assert_eq!(
            frankfurt.zone.map(|z| z.name().to_owned()).as_deref(),
            Some("Europe/Berlin")
        );
        assert!(
            frankfurt
                .wind
                .is_some_and(|w| (w.value() - 232.3).abs() < 1e-9)
        );
        // Kennedy: neither in the zone table nor with a wind downloaded.
        let kennedy = airport("KJFK").context("Kennedy")?;
        assert!(kennedy.zone.is_none() && kennedy.wind.is_none());
        assert_eq!(
            (output.zones.by_code, output.zones.missing, output.winds),
            (1, 1, 1)
        );
        // The asset reads back.
        let back = aerocosm_core::world::WorldDefinition::from_asset(&output.world.to_asset()?)?;
        assert_eq!(back, output.world);
        Ok(())
    }
}
