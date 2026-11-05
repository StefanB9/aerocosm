//! Natural Earth layers → basemap.

mod geojson;

use std::{cmp::Reverse, io::Read};

use aerocosm_basemap::{Basemap, BasemapLayers, Bounds, Country, Line, Path, Place, Polygon, Ring};
use aerocosm_core::LatLon;
use anyhow::{Context, Result};
use serde::{Deserialize, de::DeserializeOwned};

use self::geojson::{FeatureCollection, Geometry};
use crate::geometry::{self, Point, Rect};

/// One reader per Natural Earth layer (the 1:10m `GeoJSON` files).
#[derive(Debug)]
pub struct NaturalEarthSources<R> {
    /// `ne_10m_admin_0_countries`.
    pub countries: R,
    /// `ne_10m_admin_0_boundary_lines_land`.
    pub borders: R,
    /// `ne_10m_admin_1_states_provinces_lines`.
    pub state_borders: R,
    /// `ne_10m_lakes`.
    pub lakes: R,
    /// `ne_10m_rivers_lake_centerlines`.
    pub rivers: R,
    /// `ne_10m_populated_places`.
    pub places: R,
}

/// What area to cover and how much detail to keep.
#[derive(Debug, Clone)]
pub struct BasemapSettings {
    bounds: Rect,
    /// Douglas–Peucker tolerance in degrees.
    tolerance: f64,
    /// `ADM0_A3` code whose state borders are kept.
    state_country: &'static str,
}

impl BasemapSettings {
    /// Germany with a margin showing its neighbours.
    pub fn germany() -> Result<Self> {
        Ok(Self {
            bounds: Rect {
                west: 3.0,
                south: 45.5,
                east: 17.5,
                north: 56.5,
            },
            // ~150-220 m: invisible at country zoom, slightly coarse at airport
            // zoom, where OSM detail takes over in a later milestone.
            tolerance: 0.002,
            state_country: "DEU",
        })
    }

    fn basemap_bounds(&self) -> Result<Bounds> {
        let Rect {
            west,
            south,
            east,
            north,
        } = self.bounds;
        Bounds::new(LatLon::new(south, west)?, LatLon::new(north, east)?)
    }
}

#[derive(Deserialize)]
struct CountryProperties {
    #[serde(rename = "ISO_A2_EH")]
    code: String,
    #[serde(rename = "NAME")]
    name: String,
}

#[derive(Deserialize)]
struct BorderProperties {
    #[serde(rename = "SCALERANK")]
    rank: Option<u8>,
}

#[derive(Deserialize)]
struct StateBorderProperties {
    #[serde(rename = "ADM0_A3")]
    country: Option<String>,
    #[serde(rename = "SCALERANK")]
    rank: u8,
}

#[derive(Deserialize)]
struct RankProperties {
    scalerank: f64,
}

#[derive(Deserialize)]
struct PlaceProperties {
    #[serde(rename = "NAME")]
    name: String,
    #[serde(rename = "POP_MAX")]
    population: i64,
    #[serde(rename = "SCALERANK")]
    rank: u8,
}

/// Builds a basemap from Natural Earth layers: clips to the bounds, simplifies,
/// rounds coordinates to 5 decimals (~1 m) and drops features that end up
/// empty.
pub fn import_basemap<R: Read>(
    sources: NaturalEarthSources<R>,
    settings: &BasemapSettings,
) -> Result<Basemap> {
    let mut countries = Vec::new();
    for feature in read_layer::<CountryProperties>(sources.countries, "countries")? {
        let Ok(code) = feature.properties.code.parse() else {
            continue;
        };
        let polygons = polygons(feature.geometry.as_ref(), settings)?;
        if !polygons.is_empty() {
            countries.push(Country {
                code,
                name: feature.properties.name,
                polygons,
            });
        }
    }

    let mut borders = Vec::new();
    for feature in read_layer::<BorderProperties>(sources.borders, "borders")? {
        let rank = feature.properties.rank.unwrap_or(u8::MAX);
        borders.extend(lines(feature.geometry.as_ref(), rank, settings)?);
    }

    let mut state_borders = Vec::new();
    for feature in read_layer::<StateBorderProperties>(sources.state_borders, "state borders")? {
        if feature.properties.country.as_deref() == Some(settings.state_country) {
            let rank = feature.properties.rank;
            state_borders.extend(lines(feature.geometry.as_ref(), rank, settings)?);
        }
    }

    let mut lakes = Vec::new();
    for feature in read_layer::<RankProperties>(sources.lakes, "lakes")? {
        lakes.extend(polygons(feature.geometry.as_ref(), settings)?);
    }

    let mut rivers = Vec::new();
    for feature in read_layer::<RankProperties>(sources.rivers, "rivers")? {
        let rank = rank_from_f64(feature.properties.scalerank);
        rivers.extend(lines(feature.geometry.as_ref(), rank, settings)?);
    }

    let mut places = Vec::new();
    for feature in read_layer::<PlaceProperties>(sources.places, "places")? {
        let Some(Geometry::Point(point)) = feature.geometry else {
            continue;
        };
        if !settings.bounds.contains(point) {
            continue;
        }
        let population = u32::try_from(feature.properties.population).unwrap_or(0);
        let location = to_lat_lon(round(point))?;
        places.push(Place::new(
            feature.properties.name,
            location,
            population,
            feature.properties.rank,
        )?);
    }
    // Most important first, so the viewer can place labels greedily in order.
    places.sort_by(|a, b| {
        (a.rank(), Reverse(a.population()), a.name()).cmp(&(
            b.rank(),
            Reverse(b.population()),
            b.name(),
        ))
    });

    Basemap::new(BasemapLayers {
        bounds: settings.basemap_bounds()?,
        countries,
        lakes,
        borders,
        state_borders,
        rivers,
        places,
    })
    .context("imported basemap is invalid")
}

fn read_layer<P: DeserializeOwned>(
    reader: impl Read,
    layer: &str,
) -> Result<Vec<geojson::Feature<P>>> {
    let collection: FeatureCollection<P> = serde_json::from_reader(reader)
        .with_context(|| format!("cannot parse Natural Earth {layer} layer"))?;
    Ok(collection.features)
}

/// Natural Earth ranks are small non-negative numbers, some stored as floats.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rank_from_f64(rank: f64) -> u8 {
    // SAFETY: clamped to the u8 range first, so the cast cannot wrap.
    rank.clamp(0.0, f64::from(u8::MAX)).round() as u8
}

fn round([lon, lat]: Point) -> Point {
    const SCALE: f64 = 100_000.0;
    [(lon * SCALE).round() / SCALE, (lat * SCALE).round() / SCALE]
}

fn to_lat_lon([lon, lat]: Point) -> Result<LatLon> {
    LatLon::new(lat, lon)
}

fn to_path(points: &[Point]) -> Result<Path> {
    let mut path = Vec::with_capacity(points.len());
    for point in points {
        path.push(to_lat_lon(*point)?);
    }
    Ok(Path::new(path))
}

/// Clips, simplifies and rounds one ring; `None` if too little remains.
fn prepare_ring(ring: &[Point], settings: &BasemapSettings) -> Result<Option<Ring>> {
    let clipped = geometry::clip_ring(ring, &settings.bounds);
    let mut points: Vec<Point> = geometry::simplify(&clipped, settings.tolerance)
        .into_iter()
        .map(round)
        .collect();
    points.dedup();
    if points.len() < 4 {
        return Ok(None);
    }
    Ok(Some(Ring::new(to_path(&points)?)?))
}

fn polygons(geometry: Option<&Geometry>, settings: &BasemapSettings) -> Result<Vec<Polygon>> {
    let mut out = Vec::new();
    for rings in geometry.map_or_else(Vec::new, Geometry::polygons) {
        let mut prepared = Vec::with_capacity(rings.len());
        for (index, ring) in rings.iter().enumerate() {
            match prepare_ring(ring, settings)? {
                Some(ring) => prepared.push(ring),
                // Without its outer ring the holes mean nothing.
                None if index == 0 => break,
                None => {}
            }
        }
        if !prepared.is_empty() {
            out.push(Polygon::new(prepared)?);
        }
    }
    Ok(out)
}

fn lines(geometry: Option<&Geometry>, rank: u8, settings: &BasemapSettings) -> Result<Vec<Line>> {
    let mut out = Vec::new();
    for line in geometry.map_or_else(Vec::new, Geometry::lines) {
        for piece in geometry::clip_line(line, &settings.bounds) {
            let mut points: Vec<Point> = geometry::simplify(&piece, settings.tolerance)
                .into_iter()
                .map(round)
                .collect();
            points.dedup();
            if points.len() >= 2 {
                out.push(Line::new(rank, to_path(&points)?)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;

    fn collection(features: &str) -> Cursor<Vec<u8>> {
        Cursor::new(
            format!(r#"{{"type": "FeatureCollection", "features": [{features}]}}"#).into_bytes(),
        )
    }

    fn polygon(west: f64, south: f64, size: f64) -> String {
        let (east, north) = (west + size, south + size);
        format!(
            r#"{{"type": "Polygon", "coordinates": [[[{west},{south}],[{east},{south}],[{east},{north}],[{west},{north}],[{west},{south}]]]}}"#
        )
    }

    fn sources() -> NaturalEarthSources<Cursor<Vec<u8>>> {
        NaturalEarthSources {
            countries: collection(&format!(
                r#"{{"properties": {{"ISO_A2_EH": "DE", "NAME": "Germany"}}, "geometry": {}}},
                   {{"properties": {{"ISO_A2_EH": "-99", "NAME": "Nowhere"}}, "geometry": {}}},
                   {{"properties": {{"ISO_A2_EH": "FR", "NAME": "France"}}, "geometry": {}}},
                   {{"properties": {{"ISO_A2_EH": "US", "NAME": "Far away"}}, "geometry": {}}}"#,
                polygon(6.0, 48.0, 3.0),
                polygon(7.0, 49.0, 1.0),
                polygon(1.0, 46.0, 4.0),
                polygon(-100.0, 40.0, 5.0),
            )),
            borders: collection(
                r#"{"properties": {"SCALERANK": 0}, "geometry": {"type": "LineString", "coordinates": [[6.0,48.0],[6.5,49.0],[7.0,50.0]]}},
                   {"properties": {"SCALERANK": null}, "geometry": {"type": "LineString", "coordinates": [[30.0,48.0],[31.0,49.0]]}}"#,
            ),
            state_borders: collection(
                r#"{"properties": {"ADM0_A3": "DEU", "SCALERANK": 3}, "geometry": {"type": "LineString", "coordinates": [[8.0,49.0],[9.0,50.0]]}},
                   {"properties": {"ADM0_A3": "FRA", "SCALERANK": 3}, "geometry": {"type": "LineString", "coordinates": [[5.0,47.0],[5.5,47.5]]}},
                   {"properties": {"ADM0_A3": null, "SCALERANK": 3}, "geometry": null}"#,
            ),
            lakes: collection(&format!(
                r#"{{"properties": {{"scalerank": 1, "name": "Bodensee"}}, "geometry": {}}}"#,
                polygon(9.2, 47.5, 0.3)
            )),
            rivers: collection(
                r#"{"properties": {"scalerank": 2.0, "name": "Rhein"}, "geometry": {"type": "MultiLineString", "coordinates": [[[7.6,47.6],[8.0,49.0],[6.0,52.0],[2.0,53.0]]]}}"#,
            ),
            places: collection(
                r#"{"properties": {"NAME": "Bonn", "POP_MAX": 330000, "SCALERANK": 5}, "geometry": {"type": "Point", "coordinates": [7.1,50.73]}},
                   {"properties": {"NAME": "Berlin", "POP_MAX": 3406000, "SCALERANK": 1}, "geometry": {"type": "Point", "coordinates": [13.4,52.52]}},
                   {"properties": {"NAME": "Hamburg", "POP_MAX": 1757000, "SCALERANK": 1}, "geometry": {"type": "Point", "coordinates": [10.0,53.55]}},
                   {"properties": {"NAME": "Paris", "POP_MAX": 11000000, "SCALERANK": 0}, "geometry": {"type": "Point", "coordinates": [2.35,48.86]}},
                   {"properties": {"NAME": "Nowhere", "POP_MAX": -99, "SCALERANK": 9}, "geometry": {"type": "Point", "coordinates": [9.0,50.0]}}"#,
            ),
        }
    }

    #[test]
    fn test_import_basemap_filters_and_clips_layers() -> Result<()> {
        let basemap = import_basemap(sources(), &BasemapSettings::germany()?)?;
        let codes: Vec<_> = basemap
            .countries()
            .iter()
            .map(|c| c.code.as_str())
            .collect();
        assert_eq!(
            codes,
            ["DE", "FR"],
            "invalid codes and countries outside the bounds are dropped"
        );
        let france = &basemap.countries()[1].polygons[0];
        assert!(
            france
                .outer()
                .path()
                .points()
                .iter()
                .all(|p| p.longitude().value() >= 3.0),
            "clipped at the west edge"
        );
        assert_eq!(basemap.borders().len(), 1);
        assert_eq!(basemap.borders()[0].rank(), 0);
        assert_eq!(
            basemap.state_borders().len(),
            1,
            "only the simulated country's states"
        );
        assert_eq!(basemap.lakes().len(), 1);
        assert_eq!(basemap.rivers().len(), 1);
        assert_eq!(basemap.rivers()[0].rank(), 2);
        Ok(())
    }

    #[test]
    fn test_import_basemap_orders_places_by_importance() -> Result<()> {
        let basemap = import_basemap(sources(), &BasemapSettings::germany()?)?;
        let names: Vec<_> = basemap.places().iter().map(Place::name).collect();
        assert_eq!(names, ["Berlin", "Hamburg", "Bonn", "Nowhere"]);
        let nowhere = basemap
            .places()
            .iter()
            .find(|p| p.name() == "Nowhere")
            .map(Place::population);
        assert_eq!(nowhere, Some(0), "negative populations become 0");
        Ok(())
    }

    #[test]
    fn test_import_basemap_rounds_coordinates() -> Result<()> {
        let mut sources = sources();
        sources.places = collection(
            r#"{"properties": {"NAME": "Exact", "POP_MAX": 1, "SCALERANK": 1}, "geometry": {"type": "Point", "coordinates": [8.123456789,50.987654321]}}"#,
        );
        let basemap = import_basemap(sources, &BasemapSettings::germany()?)?;
        let place = basemap.places()[0].location();
        assert!((place.longitude().value() - 8.123_46).abs() < 1e-12);
        assert!((place.latitude().value() - 50.987_65).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn test_import_basemap_names_the_failing_layer() -> Result<()> {
        let mut sources = sources();
        sources.lakes = Cursor::new(b"not json".to_vec());
        let error = import_basemap(sources, &BasemapSettings::germany()?)
            .err()
            .context("must fail")?;
        assert!(format!("{error:#}").contains("lakes"), "{error:#}");
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_prepare_ring_yields_valid_rings_inside_bounds(
            mut ring in prop::collection::vec((-5.0_f64..25.0, 40.0_f64..62.0).prop_map(|(lon, lat)| [lon, lat]), 3..40),
        ) {
            ring.push(ring[0]);
            let settings = BasemapSettings::germany().map_err(|e| TestCaseError::fail(e.to_string()))?;
            let bounds = settings.basemap_bounds().map_err(|e| TestCaseError::fail(e.to_string()))?;
            let prepared = prepare_ring(&ring, &settings).map_err(|e| TestCaseError::fail(format!("{e:#}")))?;
            if let Some(prepared) = prepared {
                prop_assert!(prepared.path().points().iter().all(|p| bounds.contains(*p)));
            }
        }
    }
}
