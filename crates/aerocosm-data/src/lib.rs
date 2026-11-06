//! Offline data pipeline: turns public datasets into aerocosm's world and
//! basemap files.

mod geometry;
pub mod natural_earth;
pub mod osm;
pub mod ourairports;

use std::io::{Cursor, Read};

use anyhow::{Context, Result};

use crate::{
    natural_earth::{BasemapSettings, NaturalEarthSources, import_basemap, read_places},
    ourairports::{ImportReport, OurAirportsSources, WorldSettings, import_world},
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

    use anyhow::Result;

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
}
