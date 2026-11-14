//! The basemap: land and water meshes, and border/river lines as retained
//! gizmos.

use aerocosm_basemap::Line;
use aerocosm_core::LatLon;
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
    anchor::MapAnchor,
    camera::CameraRig,
    data::{MAP_LAYER, MapData},
    lod,
    projection::MapProjection,
    tessellate::fill_polygons,
    theme,
};

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

/// A frame for shapes spanning `points`: at the centre of their extent.
/// Natural Earth splits shapes at the antimeridian, so they are built
/// unwrapped in it.
fn frame_of<'a>(points: impl IntoIterator<Item = &'a LatLon>) -> Option<MapProjection> {
    let mut extent: Option<[f64; 4]> = None;
    for point in points {
        let (lat, lon) = (point.latitude().value(), point.longitude().value());
        let e = extent.get_or_insert([lat, lon, lat, lon]);
        *e = [e[0].min(lat), e[1].min(lon), e[2].max(lat), e[3].max(lon)];
    }
    let [south, west, north, east] = extent?;
    LatLon::new(f64::midpoint(south, north), f64::midpoint(west, east))
        .ok()
        .map(MapProjection::new)
}

fn spawn_basemap(
    mut commands: Commands,
    map: Res<MapData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut gizmos: ResMut<Assets<GizmoAsset>>,
) {
    let basemap = &map.basemap;
    let layer = RenderLayers::layer(MAP_LAYER);
    let land = materials.add(theme::LAND);
    let lake = materials.add(theme::LAKE);
    let shapes = basemap
        .countries()
        .iter()
        .flat_map(|country| &country.polygons)
        .map(|polygon| (polygon, &land, 0.0))
        .chain(basemap.lakes().iter().map(|polygon| (polygon, &lake, 0.2)));
    // One mesh per country and lake in its own frame (plan M5 D130), drawn
    // three times a world apart (D131); frustum culling drops the copies
    // out of view.
    for (polygon, material, z) in shapes {
        let Some(frame) = frame_of(polygon.outer().path().points()) else {
            continue;
        };
        match fill_polygons([polygon], &frame) {
            Ok(triangles) => {
                let mesh = meshes.add(triangles.into_mesh());
                for anchor in MapAnchor::copies(frame) {
                    commands.spawn((
                        anchor,
                        Mesh2d(mesh.clone()),
                        MeshMaterial2d(material.clone()),
                        Transform::from_xyz(0.0, 0.0, z),
                        layer.clone(),
                    ));
                }
            }
            Err(error) => error!("cannot tessellate basemap shape: {error:#}"),
        }
    }

    // Lines in one frame at the map's centre, unwrapped.
    let frame = MapProjection::zero();
    let mut lines = |lines: &mut dyn Iterator<Item = &Line>, color: Color| {
        let mut asset = GizmoAsset::new();
        for line in lines {
            asset.linestrip_2d(
                line.path()
                    .points()
                    .iter()
                    .map(|p| frame.project_unwrapped(*p)),
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
    for anchor in MapAnchor::copies(frame) {
        commands.spawn((
            anchor,
            Gizmo {
                handle: state_handle.clone(),
                line_config: line_config(1.0),
                ..default()
            },
            Transform::default(),
            layer.clone(),
        ));
    }

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
        for anchor in MapAnchor::copies(frame) {
            commands.spawn((
                anchor,
                RiverBucket {
                    rank,
                    gizmo: Gizmo {
                        handle: handle.clone(),
                        line_config: line_config(if rank <= 4 { 1.6 } else { 1.1 }),
                        ..default()
                    },
                },
                Transform::default(),
                layer.clone(),
            ));
        }
    }

    // Borders last: they read best on top of rivers and state lines.
    for anchor in MapAnchor::copies(frame) {
        commands.spawn((
            anchor,
            Gizmo {
                handle: border_handle.clone(),
                line_config: line_config(1.5),
                ..default()
            },
            Transform::default(),
            layer.clone(),
        ));
    }
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
