//! The basemap file: simplified geographic layers drawn beneath the simulation.
//!
//! Written by `aerocosm-data`, read by the viewer. Coordinates are stored as
//! compact `[lon, lat]` pairs (`GeoJSON` order) to keep the file small.

mod error;
mod path;

use aerocosm_core::{LatLon, world::codes::CountryCode};
use anyhow::{Context, Result};
pub use error::BasemapError;
pub use path::Path;
use serde::{Deserialize, Serialize};

/// Basemap file format version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Rounding slack when checking that clipped points lie within the bounds.
const BOUNDS_TOLERANCE: f64 = 1e-9;

/// The area a basemap covers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawBounds")]
pub struct Bounds {
    south_west: LatLon,
    north_east: LatLon,
}

#[derive(Deserialize)]
struct RawBounds {
    south_west: LatLon,
    north_east: LatLon,
}

impl TryFrom<RawBounds> for Bounds {
    type Error = anyhow::Error;

    fn try_from(raw: RawBounds) -> Result<Self> {
        Self::new(raw.south_west, raw.north_east)
    }
}

impl Bounds {
    /// A box from its south-west to its north-east corner; must not be empty.
    pub fn new(south_west: LatLon, north_east: LatLon) -> Result<Self> {
        if south_west.latitude() >= north_east.latitude()
            || south_west.longitude() >= north_east.longitude()
        {
            return Err(BasemapError::InvalidBounds.into());
        }
        Ok(Self {
            south_west,
            north_east,
        })
    }

    /// South-west corner.
    pub const fn south_west(&self) -> LatLon {
        self.south_west
    }

    /// North-east corner.
    pub const fn north_east(&self) -> LatLon {
        self.north_east
    }

    /// Whether `point` lies inside or on the edge of the box.
    pub fn contains(&self, point: LatLon) -> bool {
        let (lat, lon) = (point.latitude().value(), point.longitude().value());
        let (south, west) = (
            self.south_west.latitude().value(),
            self.south_west.longitude().value(),
        );
        let (north, east) = (
            self.north_east.latitude().value(),
            self.north_east.longitude().value(),
        );
        (south - BOUNDS_TOLERANCE..=north + BOUNDS_TOLERANCE).contains(&lat)
            && (west - BOUNDS_TOLERANCE..=east + BOUNDS_TOLERANCE).contains(&lon)
    }
}

/// A closed polygon boundary: at least four points, first equal to last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Path", into = "Path")]
pub struct Ring(Path);

impl TryFrom<Path> for Ring {
    type Error = anyhow::Error;

    fn try_from(path: Path) -> Result<Self> {
        Self::new(path)
    }
}

impl From<Ring> for Path {
    fn from(ring: Ring) -> Self {
        ring.0
    }
}

impl Ring {
    /// Validates that `path` is closed and has at least four points.
    pub fn new(path: Path) -> Result<Self> {
        let points = path.points();
        if points.len() < 4 || points.first() != points.last() {
            return Err(BasemapError::InvalidRing.into());
        }
        Ok(Self(path))
    }

    /// The boundary, closed.
    pub const fn path(&self) -> &Path {
        &self.0
    }
}

/// A polygon: an outer ring followed by any holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Ring>", into = "Vec<Ring>")]
pub struct Polygon(Vec<Ring>);

impl TryFrom<Vec<Ring>> for Polygon {
    type Error = anyhow::Error;

    fn try_from(rings: Vec<Ring>) -> Result<Self> {
        Self::new(rings)
    }
}

impl From<Polygon> for Vec<Ring> {
    fn from(polygon: Polygon) -> Self {
        polygon.0
    }
}

impl Polygon {
    /// Validates that there is at least an outer ring.
    pub fn new(rings: Vec<Ring>) -> Result<Self> {
        if rings.is_empty() {
            return Err(BasemapError::EmptyPolygon.into());
        }
        Ok(Self(rings))
    }

    /// The outer boundary.
    pub fn outer(&self) -> &Ring {
        // SAFETY: construction guarantees at least one ring.
        &self.0[0]
    }

    /// Holes cut out of the outer ring.
    pub fn holes(&self) -> &[Ring] {
        &self.0[1..]
    }

    /// All rings, outer first.
    pub fn rings(&self) -> &[Ring] {
        &self.0
    }
}

/// A polyline with a level-of-detail rank (lower is more important).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawLine")]
pub struct Line {
    rank: u8,
    path: Path,
}

#[derive(Deserialize)]
struct RawLine {
    rank: u8,
    path: Path,
}

impl TryFrom<RawLine> for Line {
    type Error = anyhow::Error;

    fn try_from(raw: RawLine) -> Result<Self> {
        Self::new(raw.rank, raw.path)
    }
}

impl Line {
    /// Validates that `path` has at least two points.
    pub fn new(rank: u8, path: Path) -> Result<Self> {
        if path.points().len() < 2 {
            return Err(BasemapError::InvalidLine.into());
        }
        Ok(Self { rank, path })
    }

    /// Level-of-detail rank; lower is more important.
    pub const fn rank(&self) -> u8 {
        self.rank
    }

    /// The points.
    pub const fn path(&self) -> &Path {
        &self.path
    }
}

/// A country's land area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Country {
    /// ISO code, used for styling the simulated country differently.
    pub code: CountryCode,
    /// Display name.
    pub name: String,
    /// Land polygons, clipped to the bounds.
    pub polygons: Vec<Polygon>,
}

/// A labelled settlement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawPlace")]
pub struct Place {
    name: String,
    location: LatLon,
    population: u32,
    rank: u8,
}

#[derive(Deserialize)]
struct RawPlace {
    name: String,
    location: LatLon,
    population: u32,
    rank: u8,
}

impl TryFrom<RawPlace> for Place {
    type Error = anyhow::Error;

    fn try_from(raw: RawPlace) -> Result<Self> {
        Self::new(raw.name, raw.location, raw.population, raw.rank)
    }
}

impl Place {
    /// Validates that the place has a name.
    pub fn new(name: String, location: LatLon, population: u32, rank: u8) -> Result<Self> {
        if name.trim().is_empty() {
            return Err(BasemapError::UnnamedPlace.into());
        }
        Ok(Self {
            name,
            location,
            population,
            rank,
        })
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Position.
    pub const fn location(&self) -> LatLon {
        self.location
    }

    /// Population of the metropolitan area.
    pub const fn population(&self) -> u32 {
        self.population
    }

    /// Level-of-detail rank; lower is more important.
    pub const fn rank(&self) -> u8 {
        self.rank
    }
}

/// The layers of a basemap, before cross-layer validation.
#[derive(Debug, Clone, PartialEq)]
pub struct BasemapLayers {
    /// Area covered; every point must lie within it.
    pub bounds: Bounds,
    /// Land, per country.
    pub countries: Vec<Country>,
    /// Lake water areas.
    pub lakes: Vec<Polygon>,
    /// International land borders.
    pub borders: Vec<Line>,
    /// First-level administrative borders (German states).
    pub state_borders: Vec<Line>,
    /// River centre lines.
    pub rivers: Vec<Line>,
    /// Settlement labels.
    pub places: Vec<Place>,
}

/// A validated basemap.
#[derive(Debug, Clone, PartialEq)]
pub struct Basemap {
    layers: BasemapLayers,
}

/// On-disk form of a basemap.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BasemapFile {
    format_version: u32,
    bounds: Bounds,
    countries: Vec<Country>,
    lakes: Vec<Polygon>,
    borders: Vec<Line>,
    state_borders: Vec<Line>,
    rivers: Vec<Line>,
    places: Vec<Place>,
}

impl Basemap {
    /// Validates names and that every point lies within the bounds.
    pub fn new(layers: BasemapLayers) -> Result<Self> {
        let bounds = layers.bounds;
        let inside = |path: &Path| path.points().iter().all(|p| bounds.contains(*p));
        let polygons_inside = |polygons: &[Polygon]| {
            polygons
                .iter()
                .flat_map(Polygon::rings)
                .all(|r| inside(r.path()))
        };
        let lines_inside = |lines: &[Line]| lines.iter().all(|l| inside(l.path()));
        for country in &layers.countries {
            if country.name.trim().is_empty() {
                return Err(BasemapError::UnnamedPlace.into());
            }
        }
        let checks = [
            (
                "countries",
                layers
                    .countries
                    .iter()
                    .all(|c| polygons_inside(&c.polygons)),
            ),
            ("lakes", polygons_inside(&layers.lakes)),
            ("borders", lines_inside(&layers.borders)),
            ("state_borders", lines_inside(&layers.state_borders)),
            ("rivers", lines_inside(&layers.rivers)),
            (
                "places",
                layers.places.iter().all(|p| bounds.contains(p.location)),
            ),
        ];
        if let Some((layer, _)) = checks.into_iter().find(|(_, ok)| !ok) {
            return Err(BasemapError::OutOfBounds { layer }.into());
        }
        Ok(Self { layers })
    }

    /// Parses and validates a basemap file.
    pub fn from_json(text: &str) -> Result<Self> {
        let file: BasemapFile = serde_json::from_str(text).context("malformed basemap file")?;
        if file.format_version != FORMAT_VERSION {
            return Err(BasemapError::UnsupportedFormat {
                found: file.format_version,
                supported: FORMAT_VERSION,
            }
            .into());
        }
        Self::new(BasemapLayers {
            bounds: file.bounds,
            countries: file.countries,
            lakes: file.lakes,
            borders: file.borders,
            state_borders: file.state_borders,
            rivers: file.rivers,
            places: file.places,
        })
        .context("invalid basemap file")
    }

    /// Serialises as compact JSON with a trailing newline.
    pub fn to_json(&self) -> Result<String> {
        let layers = self.layers.clone();
        let file = BasemapFile {
            format_version: FORMAT_VERSION,
            bounds: layers.bounds,
            countries: layers.countries,
            lakes: layers.lakes,
            borders: layers.borders,
            state_borders: layers.state_borders,
            rivers: layers.rivers,
            places: layers.places,
        };
        let mut json = serde_json::to_string(&file).context("cannot serialise basemap")?;
        json.push('\n');
        Ok(json)
    }

    /// Unwraps the layers.
    pub fn into_layers(self) -> BasemapLayers {
        self.layers
    }

    /// Area covered.
    pub const fn bounds(&self) -> Bounds {
        self.layers.bounds
    }

    /// Land, per country.
    pub fn countries(&self) -> &[Country] {
        &self.layers.countries
    }

    /// Lake water areas.
    pub fn lakes(&self) -> &[Polygon] {
        &self.layers.lakes
    }

    /// International land borders.
    pub fn borders(&self) -> &[Line] {
        &self.layers.borders
    }

    /// First-level administrative borders.
    pub fn state_borders(&self) -> &[Line] {
        &self.layers.state_borders
    }

    /// River centre lines.
    pub fn rivers(&self) -> &[Line] {
        &self.layers.rivers
    }

    /// Settlement labels.
    pub fn places(&self) -> &[Place] {
        &self.layers.places
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::{Context, Result};

    use super::*;

    fn point(lat: f64, lon: f64) -> Result<LatLon> {
        LatLon::new(lat, lon)
    }

    fn square(south: f64, west: f64, size: f64) -> Result<Ring> {
        Ring::new(Path::new(vec![
            point(south, west)?,
            point(south, west + size)?,
            point(south + size, west + size)?,
            point(south + size, west)?,
            point(south, west)?,
        ]))
    }

    fn bounds() -> Result<Bounds> {
        Bounds::new(point(45.0, 3.0)?, point(57.0, 18.0)?)
    }

    fn sample() -> Result<Basemap> {
        Basemap::new(BasemapLayers {
            bounds: bounds()?,
            countries: vec![Country {
                code: "DE".parse()?,
                name: "Germany".to_owned(),
                polygons: vec![Polygon::new(vec![
                    square(48.0, 8.0, 2.0)?,
                    square(48.5, 8.5, 0.5)?,
                ])?],
            }],
            lakes: vec![Polygon::new(vec![square(47.5, 9.2, 0.3)?])?],
            borders: vec![Line::new(
                1,
                Path::new(vec![point(47.0, 7.0)?, point(48.0, 7.5)?]),
            )?],
            state_borders: Vec::new(),
            rivers: vec![Line::new(
                2,
                Path::new(vec![
                    point(50.0, 8.0)?,
                    point(50.1, 8.3)?,
                    point(50.2, 8.4)?,
                ]),
            )?],
            places: vec![Place::new(
                "Frankfurt".to_owned(),
                point(50.11, 8.68)?,
                2_895_000,
                2,
            )?],
        })
    }

    fn rejection<T: std::fmt::Debug>(result: Result<T>) -> Result<BasemapError> {
        let error = result.err().context("must be rejected")?;
        error
            .downcast::<BasemapError>()
            .context("expected a BasemapError")
    }

    #[test]
    fn test_bounds_rejects_inverted() -> Result<()> {
        assert_eq!(
            rejection(Bounds::new(point(57.0, 3.0)?, point(45.0, 18.0)?))?,
            BasemapError::InvalidBounds
        );
        Ok(())
    }

    #[test]
    fn test_bounds_contains() -> Result<()> {
        let bounds = bounds()?;
        assert!(bounds.contains(point(50.0, 10.0)?));
        assert!(bounds.contains(point(45.0, 3.0)?));
        assert!(!bounds.contains(point(44.9, 10.0)?));
        assert_eq!(bounds.south_west(), point(45.0, 3.0)?);
        assert_eq!(bounds.north_east(), point(57.0, 18.0)?);
        Ok(())
    }

    #[test]
    fn test_ring_requires_closed_path_of_four_points() -> Result<()> {
        let open = Path::new(vec![
            point(0.0, 0.0)?,
            point(0.0, 1.0)?,
            point(1.0, 1.0)?,
            point(1.0, 0.0)?,
        ]);
        assert_eq!(rejection(Ring::new(open))?, BasemapError::InvalidRing);
        let short = Path::new(vec![point(0.0, 0.0)?, point(0.0, 1.0)?, point(0.0, 0.0)?]);
        assert_eq!(rejection(Ring::new(short))?, BasemapError::InvalidRing);
        assert_eq!(square(0.0, 0.0, 1.0)?.path().points().len(), 5);
        Ok(())
    }

    #[test]
    fn test_polygon_requires_outer_ring() -> Result<()> {
        assert_eq!(
            rejection(Polygon::new(Vec::new()))?,
            BasemapError::EmptyPolygon
        );
        let polygon = Polygon::new(vec![square(0.0, 0.0, 2.0)?, square(0.5, 0.5, 0.5)?])?;
        assert_eq!(polygon.outer().path().points().len(), 5);
        assert_eq!(polygon.holes().len(), 1);
        Ok(())
    }

    #[test]
    fn test_line_requires_two_points() -> Result<()> {
        assert_eq!(
            rejection(Line::new(1, Path::new(vec![point(0.0, 0.0)?])))?,
            BasemapError::InvalidLine
        );
        let line = Line::new(3, Path::new(vec![point(0.0, 0.0)?, point(1.0, 1.0)?]))?;
        assert_eq!(line.rank(), 3);
        assert_eq!(line.path().points().len(), 2);
        Ok(())
    }

    #[test]
    fn test_place_requires_name() -> Result<()> {
        assert_eq!(
            rejection(Place::new(String::new(), point(0.0, 0.0)?, 1, 1))?,
            BasemapError::UnnamedPlace
        );
        let place = Place::new("Bonn".to_owned(), point(50.7, 7.1)?, 330_000, 5)?;
        assert_eq!(
            (place.name(), place.population(), place.rank()),
            ("Bonn", 330_000, 5)
        );
        assert_eq!(place.location(), point(50.7, 7.1)?);
        Ok(())
    }

    #[test]
    fn test_basemap_rejects_points_outside_bounds() -> Result<()> {
        let mut layers = sample()?.into_layers();
        layers
            .lakes
            .push(Polygon::new(vec![square(60.0, 9.0, 0.3)?])?);
        assert_eq!(
            rejection(Basemap::new(layers))?,
            BasemapError::OutOfBounds { layer: "lakes" }
        );
        Ok(())
    }

    #[test]
    fn test_basemap_accessors() -> Result<()> {
        let basemap = sample()?;
        assert_eq!(basemap.countries().len(), 1);
        assert_eq!(basemap.countries()[0].code.as_str(), "DE");
        assert_eq!(basemap.lakes().len(), 1);
        assert_eq!(basemap.borders().len(), 1);
        assert_eq!(basemap.state_borders().len(), 0);
        assert_eq!(basemap.rivers().len(), 1);
        assert_eq!(basemap.places().len(), 1);
        assert_eq!(basemap.bounds(), bounds()?);
        Ok(())
    }

    #[test]
    fn test_basemap_json_round_trip_is_compact() -> Result<()> {
        let basemap = sample()?;
        let json = basemap.to_json()?;
        assert!(
            json.contains("[8.0,48.0]"),
            "coordinates as [lon,lat]: {json}"
        );
        assert!(json.ends_with('\n'));
        assert_eq!(Basemap::from_json(&json)?, basemap);
        Ok(())
    }

    #[test]
    fn test_basemap_from_json_rejects_bad_version_and_invalid_content() -> Result<()> {
        let json = sample()?.to_json()?;
        let future = json.replacen("\"format_version\":1", "\"format_version\":2", 1);
        assert_eq!(
            rejection(Basemap::from_json(&future))?,
            BasemapError::UnsupportedFormat {
                found: 2,
                supported: FORMAT_VERSION
            }
        );
        let open_ring = json.replacen("[8.0,48.0]]", "[8.0,48.1]]", 1);
        assert!(Basemap::from_json(&open_ring).is_err());
        Ok(())
    }

    /// The generated basemap shipped with the project (see
    /// docs/data-sources.md).
    const GERMANY: &str = include_str!("../../../assets/basemap/germany.basemap.json");

    #[test]
    fn test_shipped_germany_basemap_is_valid() -> Result<()> {
        let basemap = Basemap::from_json(GERMANY).context("assets/basemap/germany.basemap.json")?;
        assert!(basemap.countries().iter().any(|c| c.code.as_str() == "DE"));
        assert!(basemap.places().iter().any(|p| p.name() == "Berlin"));
        assert!(basemap.state_borders().len() > 10);
        Ok(())
    }
}
