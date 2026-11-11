//! Airport grounds at airport zoom (plan M4 D87): aprons as one mesh built at
//! start-up, taxiway centrelines and stands as retained gizmos per airport,
//! switched on only below the zoom threshold and for airports in view.

pub(crate) mod geometry;

use aerocosm_core::ground::{AirportGround, EdgeKind, GroundNetwork};
use bevy::{
    camera::visibility::RenderLayers,
    gizmos::{
        GizmoAsset,
        config::{GizmoLineConfig, GizmoLineJoint},
        retained::Gizmo,
    },
    prelude::*,
};
use geometry::{NamedEdge, bounds, ground_visible, in_view, polyline_middle, stand_marker};

use crate::{
    camera::{CameraRig, CameraSet, PointerState},
    data::OVERLAY_LAYER,
    projection::MapProjection,
    tessellate::fill_outlines,
    theme,
};

/// Stand markers are this wide, m.
const STAND_MARKER: f64 = 12.0;

/// The mapped airports' ground layouts, as rendered.
#[derive(Resource)]
pub(crate) struct GroundView {
    pub(crate) ground: GroundNetwork,
}

/// One airport's taxiway or stand lines, shown while it is in view at
/// airport zoom.
#[derive(Component)]
struct GroundLines {
    bounds: Rect,
    gizmo: Gizmo,
}

/// The apron mesh.
#[derive(Component)]
struct Aprons;

/// Draws airport grounds.
pub(crate) struct GroundPlugin;

impl Plugin for GroundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ground)
            .add_systems(Update, ground_level_of_detail.after(CameraSet));
    }
}

/// `layout`'s named taxiways and taxilanes, projected, for label anchors.
pub(crate) fn named_edges(layout: &AirportGround, projection: &MapProjection) -> Vec<NamedEdge> {
    layout
        .edges()
        .iter()
        .filter(|edge| matches!(edge.kind(), EdgeKind::Taxiway | EdgeKind::Taxilane))
        .filter_map(|edge| {
            let points: Vec<Vec2> = edge
                .points()
                .iter()
                .map(|p| projection.project(*p))
                .collect();
            let length = points.windows(2).map(|w| w[0].distance(w[1])).sum();
            Some(NamedEdge {
                name: edge.name()?,
                from: edge.from().index(),
                to: edge.to().index(),
                length,
                middle: polyline_middle(&points)?,
            })
        })
        .collect()
}

fn spawn_ground(
    mut commands: Commands,
    view: Res<GroundView>,
    projection: Res<MapProjection>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut gizmos: ResMut<Assets<GizmoAsset>>,
) {
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    let outlines = view.ground.airports().iter().flat_map(|layout| {
        layout
            .aprons()
            .iter()
            .map(|ring| ring.iter().map(|p| projection.project(*p)).collect())
    });
    match fill_outlines(outlines) {
        Ok(triangles) => {
            commands.spawn((
                Aprons,
                Mesh2d(meshes.add(triangles.into_mesh())),
                MeshMaterial2d(materials.add(theme::APRON)),
                // Below the runways (z 5).
                Transform::from_xyz(0.0, 0.0, 4.0),
                Visibility::Hidden,
                layer.clone(),
            ));
        }
        Err(error) => error!("cannot tessellate aprons: {error:#}"),
    }
    let line_config = |width: f32| GizmoLineConfig {
        width,
        joints: GizmoLineJoint::Round(4),
        ..default()
    };
    for layout in view.ground.airports() {
        let project = |p: &aerocosm_core::LatLon| projection.project(*p);
        let Some(area) = bounds(layout.nodes().iter().map(|n| project(&n.position()))) else {
            continue;
        };
        let mut taxiways = GizmoAsset::new();
        let mut stands = GizmoAsset::new();
        for edge in layout.edges() {
            let points = edge.points().iter().map(project);
            match edge.kind() {
                EdgeKind::Taxiway | EdgeKind::Taxilane => {
                    taxiways.linestrip_2d(points, theme::TAXIWAY);
                }
                EdgeKind::LeadIn => stands.linestrip_2d(points, theme::STAND),
                EdgeKind::Runway => {}
            }
        }
        for stand in layout.stands() {
            let position = layout.nodes()[stand.node().as_usize()].position();
            // SAFETY: map units per metre near Germany are about 1.6.
            #[allow(clippy::cast_possible_truncation)]
            let size = (STAND_MARKER * MapProjection::units_per_meter(position)) as f32;
            #[allow(clippy::cast_possible_truncation)]
            let heading = stand.heading().value() as f32;
            for [a, b] in stand_marker(projection.project(position), heading, size) {
                stands.line_2d(a, b, theme::STAND);
            }
        }
        for (asset, width) in [(taxiways, 1.2), (stands, 1.0)] {
            commands.spawn((
                GroundLines {
                    bounds: area,
                    gizmo: Gizmo {
                        handle: gizmos.add(asset),
                        line_config: line_config(width),
                        ..default()
                    },
                },
                Transform::default(),
                layer.clone(),
            ));
        }
    }
}

/// Shows the ground below the zoom threshold, and each airport's lines only
/// while it is in view. Retained gizmos ignore `Visibility`, so their
/// `Gizmo` is added and removed (as for rivers).
fn ground_level_of_detail(
    mut commands: Commands,
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    mut aprons: Query<&mut Visibility, With<Aprons>>,
    lines: Query<(Entity, &GroundLines, Has<Gizmo>)>,
) {
    let Some(rig) = rig else {
        return;
    };
    let visible = ground_visible(rig.state.scale());
    for mut visibility in &mut aprons {
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let view = Rect::from_corners(
        rig.state.screen_to_world(Vec2::ZERO, pointer.viewport),
        rig.state
            .screen_to_world(pointer.viewport, pointer.viewport),
    );
    for (entity, ground, shown) in &lines {
        let show = visible && in_view(ground.bounds, view);
        if show && !shown {
            commands.entity(entity).insert(ground.gizmo.clone());
        } else if !show && shown {
            commands.entity(entity).remove::<Gizmo>();
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::{bounds, geometry, named_edges};
    use crate::{data, projection::MapProjection};

    #[test]
    fn test_named_edges_give_frankfurt_its_taxiway_labels() -> Result<()> {
        let (scenario, _) = data::load()?;
        let world = scenario.world();
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let layout = scenario.ground().airport(frankfurt).context("FRA layout")?;
        let projection = MapProjection::germany()?;
        let edges = named_edges(layout, &projection);
        assert!(edges.len() > 500, "{}", edges.len());
        let anchors = geometry::stretch_anchors(&edges);
        // Fewer labels than named edges, but every name has one.
        assert!(anchors.len() < edges.len() / 2, "{}", anchors.len());
        let mut names: Vec<_> = edges.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert!(names.iter().all(|n| anchors.iter().any(|(a, _)| a == n)));
        let area = bounds(
            layout
                .nodes()
                .iter()
                .map(|n| projection.project(n.position())),
        )
        .context("bounds")?;
        assert!(anchors.iter().all(|(_, at)| area.contains(*at)));
        Ok(())
    }
}
