//! The subset of `GeoJSON` that Natural Earth layers use.

use serde::Deserialize;

use crate::geometry::Point;

/// A `FeatureCollection` whose features all have properties of type `P`.
#[derive(Debug, Deserialize)]
pub(super) struct FeatureCollection<P> {
    pub(super) features: Vec<Feature<P>>,
}

/// One feature; unknown properties are ignored.
#[derive(Debug, Deserialize)]
pub(super) struct Feature<P> {
    pub(super) properties: P,
    pub(super) geometry: Option<Geometry>,
}

/// The geometry types Natural Earth uses.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "coordinates")]
pub(super) enum Geometry {
    Point(Point),
    LineString(Vec<Point>),
    MultiLineString(Vec<Vec<Point>>),
    Polygon(Vec<Vec<Point>>),
    MultiPolygon(Vec<Vec<Vec<Point>>>),
}

impl Geometry {
    /// All line strings, flattening `MultiLineString`; empty for other types.
    pub(super) fn lines(&self) -> Vec<&[Point]> {
        match self {
            Self::LineString(line) => vec![line.as_slice()],
            Self::MultiLineString(lines) => lines.iter().map(Vec::as_slice).collect(),
            _ => Vec::new(),
        }
    }

    /// All polygons as ring lists, flattening `MultiPolygon`; empty for other
    /// types.
    pub(super) fn polygons(&self) -> Vec<&[Vec<Point>]> {
        match self {
            Self::Polygon(rings) => vec![rings.as_slice()],
            Self::MultiPolygon(polygons) => polygons.iter().map(Vec::as_slice).collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct Named {
        name: String,
    }

    #[test]
    fn test_feature_collection_parses_typed_properties_and_geometries() -> Result<()> {
        let json = r#"{
            "type": "FeatureCollection", "name": "test", "crs": {}, "bbox": [0, 0, 1, 1],
            "features": [
                {"type": "Feature", "properties": {"name": "a", "extra": 1},
                 "geometry": {"type": "Point", "coordinates": [8.5, 50.0]}},
                {"type": "Feature", "properties": {"name": "b"},
                 "geometry": {"type": "MultiLineString", "coordinates": [[[0, 0], [1, 1]]]}},
                {"type": "Feature", "properties": {"name": "c"}, "geometry": null}
            ]
        }"#;
        let collection: FeatureCollection<Named> = serde_json::from_str(json).context("parse")?;
        assert_eq!(collection.features.len(), 3);
        assert_eq!(collection.features[0].properties.name, "a");
        assert!(matches!(
            collection.features[0].geometry,
            Some(Geometry::Point([8.5, 50.0]))
        ));
        assert!(
            matches!(&collection.features[1].geometry, Some(Geometry::MultiLineString(lines)) if lines.len() == 1)
        );
        assert!(collection.features[2].geometry.is_none());
        Ok(())
    }

    #[test]
    fn test_geometry_lines_and_polygons_flatten_multi_variants() -> Result<()> {
        let multi: Geometry = serde_json::from_str(
            r#"{"type": "MultiPolygon", "coordinates": [[[[0,0],[1,0],[1,1],[0,0]]], [[[2,2],[3,2],[3,3],[2,2]]]]}"#,
        )
        .context("parse")?;
        assert_eq!(multi.polygons().len(), 2);
        assert_eq!(multi.lines().len(), 0);
        let line: Geometry =
            serde_json::from_str(r#"{"type": "LineString", "coordinates": [[0,0],[1,1]]}"#)
                .context("parse")?;
        assert_eq!(line.lines().len(), 1);
        assert_eq!(line.polygons().len(), 0);
        Ok(())
    }
}
