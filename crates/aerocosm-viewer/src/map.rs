//! The basemap: land and water meshes, and border/river lines as retained
//! gizmos.

use aerocosm_basemap::Line;
use bevy::{
    camera::visibility::RenderLayers,
    gizmos::{
        GizmoAsset,
        config::{GizmoLineConfig, GizmoLineJoint},
        retained::Gizmo,
    },
    prelude::*,
};

use crate::{
    camera::CameraRig,
    data::{MAP_LAYER, MapData},
    lod,
    projection::MapProjection,
    tessellate::fill_polygons,
    theme,
};

/// Country code drawn as the simulated country.
const HOME_COUNTRY: &str = "DE";

/// Draws the basemap.
pub(crate) struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_basemap)
            .add_systems(Update, river_level_of_detail);
    }
}

/// A bucket of rivers shown below a zoom threshold.
#[derive(Component)]
struct RiverBucket {
    rank: u8,
    gizmo: Gizmo,
}

fn spawn_basemap(
    mut commands: Commands,
    map: Res<MapData>,
    projection: Res<MapProjection>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut gizmos: ResMut<Assets<GizmoAsset>>,
) {
    let basemap = &map.basemap;
    let layer = RenderLayers::layer(MAP_LAYER);
    let mut fill =
        |polygons: Vec<&aerocosm_basemap::Polygon>, color: Color, z: f32| match fill_polygons(
            polygons,
            &projection,
        ) {
            Ok(triangles) => {
                commands.spawn((
                    Mesh2d(meshes.add(triangles.into_mesh())),
                    MeshMaterial2d(materials.add(color)),
                    Transform::from_xyz(0.0, 0.0, z),
                    layer.clone(),
                ));
            }
            Err(error) => error!("cannot tessellate basemap layer: {error:#}"),
        };
    let (home, abroad): (Vec<_>, Vec<_>) = basemap
        .countries()
        .iter()
        .partition(|country| country.code.as_str() == HOME_COUNTRY);
    fill(
        abroad.iter().flat_map(|c| &c.polygons).collect(),
        theme::LAND,
        0.0,
    );
    fill(
        home.iter().flat_map(|c| &c.polygons).collect(),
        theme::LAND_HOME,
        0.1,
    );
    fill(basemap.lakes().iter().collect(), theme::LAKE, 0.2);

    let mut lines = |lines: &mut dyn Iterator<Item = &Line>, color: Color| {
        let mut asset = GizmoAsset::new();
        for line in lines {
            asset.linestrip_2d(
                line.path().points().iter().map(|p| projection.project(*p)),
                color,
            );
        }
        gizmos.add(asset)
    };
    let line_config = |width: f32| GizmoLineConfig {
        width,
        joints: GizmoLineJoint::Round(4),
        ..default()
    };
    let state_handle = lines(&mut basemap.state_borders().iter(), theme::STATE_BORDER);
    let border_handle = lines(&mut basemap.borders().iter(), theme::BORDER);
    commands.spawn((
        Gizmo {
            handle: state_handle,
            line_config: line_config(1.0),
            ..default()
        },
        layer.clone(),
    ));

    // Rivers are grouped by rank so detail appears progressively when zooming
    // in.
    let mut ranks: Vec<u8> = basemap.rivers().iter().map(Line::rank).collect();
    ranks.sort_unstable();
    ranks.dedup();
    for rank in ranks {
        let handle = lines(
            &mut basemap.rivers().iter().filter(|r| r.rank() == rank),
            theme::RIVER,
        );
        commands.spawn((
            RiverBucket {
                rank,
                gizmo: Gizmo {
                    handle,
                    line_config: line_config(if rank <= 4 { 1.6 } else { 1.1 }),
                    ..default()
                },
            },
            Transform::default(),
            layer.clone(),
        ));
    }

    // Borders last: they read best on top of rivers and state lines.
    commands.spawn((
        Gizmo {
            handle: border_handle,
            line_config: line_config(1.5),
            ..default()
        },
        layer,
    ));
}

/// Retained gizmos ignore `Visibility`, so buckets are switched by adding or
/// removing the `Gizmo` component; this only happens when crossing a zoom
/// threshold.
fn river_level_of_detail(
    mut commands: Commands,
    rig: Option<Res<CameraRig>>,
    buckets: Query<(Entity, &RiverBucket, Has<Gizmo>)>,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    for (entity, bucket, shown) in &buckets {
        let visible = lod::river_visible(bucket.rank, scale);
        if visible && !shown {
            commands.entity(entity).insert(bucket.gizmo.clone());
        } else if !visible && shown {
            commands.entity(entity).remove::<Gizmo>();
        }
    }
}
