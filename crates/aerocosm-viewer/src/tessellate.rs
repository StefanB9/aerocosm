//! Turning basemap polygons into triangle meshes (once, at startup).

use aerocosm_basemap::Polygon;
use anyhow::{Context, Result};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, Mesh, PrimitiveTopology},
};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers, math::point,
    path::Path as LyonPath,
};

use crate::projection::MapProjection;

/// Triangles in local map units.
#[derive(Debug, Default)]
pub(crate) struct Triangles {
    pub(crate) positions: Vec<[f32; 2]>,
    pub(crate) indices: Vec<u32>,
}

/// Projects and triangulates polygons (holes via the even-odd rule) into one
/// buffer.
pub(crate) fn fill_polygons<'a>(
    polygons: impl IntoIterator<Item = &'a Polygon>,
    projection: &MapProjection,
) -> Result<Triangles> {
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    for polygon in polygons {
        let mut builder = LyonPath::builder();
        for ring in polygon.rings() {
            let mut points = ring.path().points().iter().map(|p| projection.project(*p));
            let Some(first) = points.next() else {
                continue;
            };
            builder.begin(point(first.x, first.y));
            // Rings are closed; the repeated last point is implied by
            // `end(true)`.
            for p in points.take(ring.path().points().len().saturating_sub(2)) {
                builder.line_to(point(p.x, p.y));
            }
            builder.end(true);
        }
        tessellator
            .tessellate_path(
                &builder.build(),
                &FillOptions::default(),
                &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
                    vertex.position().to_array()
                }),
            )
            .context("cannot tessellate polygon")?;
    }
    Ok(Triangles {
        positions: buffers.vertices,
        indices: buffers.indices,
    })
}

impl Triangles {
    /// A render mesh at `z = 0`.
    pub(crate) fn into_mesh(self) -> Mesh {
        let positions: Vec<[f32; 3]> = self.positions.iter().map(|[x, y]| [*x, *y, 0.0]).collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_basemap::{Path, Polygon, Ring};
    use aerocosm_core::LatLon;
    use anyhow::Result;
    use bevy::math::Vec2;
    use proptest::prelude::*;

    use super::*;
    use crate::projection::MapProjection;

    fn ring(points: &[(f64, f64)]) -> Result<Ring> {
        let mut path = Vec::with_capacity(points.len() + 1);
        for (lat, lon) in points {
            path.push(LatLon::new(*lat, *lon)?);
        }
        path.push(path[0]);
        Ring::new(Path::new(path))
    }

    fn area(triangles: &Triangles) -> f64 {
        triangles
            .indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|i| Vec2::from(triangles.positions[i as usize]));
                f64::from((b - a).perp_dot(c - a).abs()) / 2.0
            })
            .sum()
    }

    fn shoelace(points: &[Vec2]) -> f64 {
        let n = points.len();
        (0..n)
            .map(|i| f64::from(points[i].perp_dot(points[(i + 1) % n])))
            .sum::<f64>()
            .abs()
            / 2.0
    }

    #[test]
    fn test_fill_square_makes_two_triangles() -> Result<()> {
        let projection = MapProjection::germany()?;
        let square = Polygon::new(vec![ring(&[
            (50.0, 9.0),
            (50.0, 10.0),
            (51.0, 10.0),
            (51.0, 9.0),
        ])?])?;
        let triangles = fill_polygons([&square], &projection)?;
        assert_eq!(triangles.indices.len(), 6);
        assert_eq!(triangles.positions.len(), 4);
        Ok(())
    }

    #[test]
    fn test_fill_respects_holes() -> Result<()> {
        let projection = MapProjection::germany()?;
        let outer = [(50.0, 9.0), (50.0, 11.0), (52.0, 11.0), (52.0, 9.0)];
        let hole = [(50.5, 9.5), (50.5, 10.5), (51.5, 10.5), (51.5, 9.5)];
        let solid = fill_polygons([&Polygon::new(vec![ring(&outer)?])?], &projection)?;
        let holed = fill_polygons(
            [&Polygon::new(vec![ring(&outer)?, ring(&hole)?])?],
            &projection,
        )?;
        let hole_only = fill_polygons([&Polygon::new(vec![ring(&hole)?])?], &projection)?;
        let expected = area(&solid) - area(&hole_only);
        assert!((area(&holed) - expected).abs() / expected < 1e-4);
        Ok(())
    }

    #[test]
    fn test_fill_merges_polygons_with_offset_indices() -> Result<()> {
        let projection = MapProjection::germany()?;
        let a = Polygon::new(vec![ring(&[(50.0, 9.0), (50.0, 10.0), (51.0, 10.0)])?])?;
        let b = Polygon::new(vec![ring(&[(52.0, 9.0), (52.0, 10.0), (53.0, 10.0)])?])?;
        let triangles = fill_polygons([&a, &b], &projection)?;
        assert_eq!(triangles.positions.len(), 6);
        assert_eq!(triangles.indices.len(), 6);
        assert!(
            triangles.indices[3..].iter().all(|i| *i >= 3),
            "second polygon indexes its own vertices"
        );
        Ok(())
    }

    #[test]
    fn test_triangles_to_mesh_keeps_counts() -> Result<()> {
        let projection = MapProjection::germany()?;
        let square = Polygon::new(vec![ring(&[
            (50.0, 9.0),
            (50.0, 10.0),
            (51.0, 10.0),
            (51.0, 9.0),
        ])?])?;
        let mesh = fill_polygons([&square], &projection)?.into_mesh();
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().map(bevy::mesh::Indices::len), Some(6));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_fill_area_matches_convex_polygon(
            sides in 3_usize..24,
            radius in 0.05_f64..2.0,
            lat in 47.0_f64..55.0,
            lon in 6.0_f64..14.0,
            rotation in 0.0_f64..std::f64::consts::TAU,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(e.to_string());
            let projection = MapProjection::germany().map_err(fail)?;
            #[allow(clippy::cast_precision_loss)]
            let points: Vec<(f64, f64)> = (0..sides)
                .map(|i| {
                    let angle = rotation + std::f64::consts::TAU * i as f64 / sides as f64;
                    (lat + radius * angle.sin(), lon + radius * angle.cos())
                })
                .collect();
            let polygon = Polygon::new(vec![ring(&points).map_err(fail)?]).map_err(fail)?;
            let triangles = fill_polygons([&polygon], &projection).map_err(fail)?;
            let projected: Vec<Vec2> = points
                .iter()
                .map(|(lat, lon)| LatLon::new(*lat, *lon).map(|p| projection.project(p)))
                .collect::<Result<_>>()
                .map_err(fail)?;
            let expected = shoelace(&projected);
            prop_assert!((area(&triangles) - expected).abs() / expected < 1e-3, "{} vs {expected}", area(&triangles));
        }
    }
}
