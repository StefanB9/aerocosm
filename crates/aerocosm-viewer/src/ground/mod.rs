//! Airport grounds at airport zoom (plan M4 D87, M5 D132): aprons, taxiway
//! centrelines, stands and their names, built per airport when it comes into
//! view at airport zoom and dropped when it is far away (`stream`), each in
//! its own frame.

pub(crate) mod geometry;
mod route;
mod stream;

use aerocosm_core::{
    Id,
    ground::{AirportGround, EdgeKind},
    world::Airport,
};
use bevy::{
    camera::visibility::RenderLayers,
    ecs::schedule::common_conditions::resource_added,
    gizmos::{
        GizmoAsset,
        config::{GizmoLineConfig, GizmoLineJoint},
        retained::Gizmo,
    },
    prelude::*,
};
use geometry::{NamedEdge, bounds, polyline_middle, stand_marker};

use crate::{
    anchor::{AnchorSet, MapAnchor},
    camera::{CameraRig, CameraSet, PointerState},
    data::{Fonts, OVERLAY_LAYER},
    driver::{SimDriver, SimReady},
    projection::MapProjection,
    tessellate::fill_outlines,
    theme,
    ui::labels::{LabelOrder, LabelRoot, forget_labels, spawn_ground_labels},
};

/// Stand markers are this wide, m.
const STAND_MARKER: f64 = 12.0;

/// The asset stores a ground is drawn into.
type DrawAssets<'w> = (
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<ColorMaterial>>,
    ResMut<'w, Assets<GizmoAsset>>,
);

/// Every mapped airport's frame and extent in it, for streaming.
#[derive(Resource, Default)]
struct GroundIndex(Vec<(Id<Airport>, MapProjection, Rect)>);

/// One airport's built ground: its meshes, lines and labels.
#[derive(Component)]
struct BuiltGround {
    index: usize,
    labels: Vec<Entity>,
}

/// Draws airport grounds.
pub(crate) struct GroundPlugin;

impl Plugin for GroundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroundIndex>().add_systems(
            Update,
            (
                index_ground.run_if(resource_added::<SimDriver>),
                stream_ground.after(AnchorSet).after(index_ground),
                route::draw_taxi_route
                    .after(CameraSet)
                    .after(crate::driver::SimSet),
            )
                .in_set(SimReady),
        );
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

fn index_ground(driver: Res<SimDriver>, mut index: ResMut<GroundIndex>) {
    index.0 = driver
        .simulation()
        .scenario()
        .ground()
        .airports()
        .iter()
        .filter_map(|layout| {
            let frame = MapProjection::new(layout.nodes().first()?.position());
            let area = bounds(layout.nodes().iter().map(|n| frame.project(n.position())))?;
            Some((layout.airport(), frame, area))
        })
        .collect();
}

/// Builds the grounds coming into view and drops those far away.
fn stream_ground(
    mut commands: Commands,
    driver: Res<SimDriver>,
    index: Res<GroundIndex>,
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    projection: Res<MapProjection>,
    fonts: Res<Fonts>,
    root: Option<Res<LabelRoot>>,
    mut order: ResMut<LabelOrder>,
    built: Query<(Entity, &BuiltGround)>,
    mut assets: DrawAssets<'_>,
) {
    let (Some(rig), Some(root)) = (rig, root) else {
        return;
    };
    let centre = rig.state.center();
    let view = Rect::from_corners(
        rig.state.screen_to_world(Vec2::ZERO, pointer.viewport),
        rig.state
            .screen_to_world(pointer.viewport, pointer.viewport),
    );
    let areas = index.0.iter().enumerate().map(|(i, (_, frame, area))| {
        let at = MapAnchor::new(*frame).place(&projection, centre);
        (i, Rect::from_corners(area.min + at, area.max + at))
    });
    let is_built = |i: usize| built.iter().any(|(_, b)| b.index == i);
    let plan = stream::plan(areas, view, rig.state.scale(), is_built);
    for (entity, ground) in &built {
        if plan.drop.contains(&ground.index) {
            forget_labels(&mut order, &ground.labels);
            for label in &ground.labels {
                commands.entity(*label).despawn();
            }
            commands.entity(entity).despawn();
        }
    }
    let ground = driver.simulation().scenario().ground();
    for i in plan.build {
        let Some((airport, frame, _)) = index.0.get(i) else {
            continue;
        };
        let Some(layout) = ground.airport(*airport) else {
            continue;
        };
        let labels = spawn_ground_labels(
            &mut commands,
            &fonts,
            &projection,
            &root,
            &mut order,
            layout,
            frame,
        );
        spawn_layout(
            &mut commands,
            &mut assets,
            layout,
            *frame,
            BuiltGround { index: i, labels },
        );
    }
}

/// Spawns the aprons, taxiways and stands of `layout` in `frame`.
fn spawn_layout(
    commands: &mut Commands,
    (meshes, materials, gizmos): &mut DrawAssets<'_>,
    layout: &AirportGround,
    frame: MapProjection,
    built: BuiltGround,
) {
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    let line_config = |width: f32| GizmoLineConfig {
        width,
        joints: GizmoLineJoint::Round(4),
        ..default()
    };
    let project = |p: &aerocosm_core::LatLon| frame.project(*p);
    let parent = commands
        .spawn((
            built,
            MapAnchor::new(frame),
            Transform::default(),
            Visibility::Inherited,
        ))
        .id();
    let outlines = layout
        .aprons()
        .iter()
        .map(|ring| ring.iter().map(project).collect());
    match fill_outlines(outlines) {
        Ok(triangles) => {
            commands.spawn((
                Mesh2d(meshes.add(triangles.into_mesh())),
                MeshMaterial2d(materials.add(theme::APRON)),
                // Below the runways (z 5).
                Transform::from_xyz(0.0, 0.0, 4.0),
                layer.clone(),
                ChildOf(parent),
            ));
        }
        Err(error) => error!("cannot tessellate aprons: {error:#}"),
    }
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
        // SAFETY: map units per metre are a few at most below 80°.
        #[allow(clippy::cast_possible_truncation)]
        let size = (STAND_MARKER * MapProjection::units_per_meter(position)) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let heading = stand.heading().value() as f32;
        for [a, b] in stand_marker(frame.project(position), heading, size) {
            stands.line_2d(a, b, theme::STAND);
        }
    }
    for (asset, width) in [(taxiways, 1.2), (stands, 1.0)] {
        commands.spawn((
            Gizmo {
                handle: gizmos.add(asset),
                line_config: line_config(width),
                ..default()
            },
            Transform::default(),
            layer.clone(),
            ChildOf(parent),
        ));
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::{Context, Result};

    use super::{bounds, geometry, named_edges};
    use crate::{data, projection::MapProjection};

    #[test]
    fn test_named_edges_give_frankfurt_its_taxiway_labels() -> Result<()> {
        let (scenario, _) = data::load_with_ground(&['E'])?;
        let world = scenario.world();
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let layout = scenario.ground().airport(frankfurt).context("FRA layout")?;
        let projection = MapProjection::new(LatLon::new(50.03, 8.57)?);
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
