//! Airport symbols, runways drawn to scale, and hover/selection.

use std::collections::HashMap;

use aerocosm_core::{
    Id, LatLon,
    world::{Airport, AirportKind, Runway, Surface, World},
};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use crate::{
    anchor::{AnchorSet, MapAnchor},
    camera::CameraRig,
    data::OVERLAY_LAYER,
    lod,
    projection::MapProjection,
    selection::Selection,
    theme,
};

/// Selection ring radius, relative to the symbol radius.
pub(crate) const SELECTED_RING: f32 = 2.4;
/// Halo around a symbol, relative to its radius, for contrast against lines.
const HALO: f32 = 1.45;

/// The simulated world, as rendered.
#[derive(Resource)]
pub(crate) struct WorldView {
    pub(crate) world: World,
}

/// An airport symbol.
#[derive(Component)]
pub(crate) struct AirportSymbol {
    pub(crate) id: Id<Airport>,
    pub(crate) kind: AirportKind,
    pub(crate) position: Vec2,
}

/// The two runway meshes (paved and soft surfaces).
#[derive(Component)]
struct RunwayLayer;

/// Draws airports and runways and handles selection.
pub(crate) struct AirportsPlugin;

impl Plugin for AirportsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .add_systems(Startup, (spawn_airports, spawn_runways))
            .add_systems(
                Update,
                (
                    update_symbols.after(AnchorSet),
                    update_runways,
                    crate::selection::hover_and_select.after(update_symbols),
                    draw_selection,
                ),
            );
    }
}

/// Drawing order of airport kinds: bigger airports draw above smaller ones.
fn kind_rank(kind: AirportKind) -> u8 {
    match kind {
        AirportKind::Airfield => 0,
        AirportKind::Regional => 1,
        AirportKind::Edge => 2,
        AirportKind::Major => 3,
    }
}

fn kind_color(kind: AirportKind) -> Color {
    match kind {
        AirportKind::Major | AirportKind::Edge => theme::ACCENT,
        AirportKind::Regional => theme::REGIONAL,
        AirportKind::Airfield => theme::AIRFIELD,
    }
}

fn spawn_airports(
    mut commands: Commands,
    view: Res<WorldView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let disc = meshes.add(Circle::new(1.0).mesh().resolution(32));
    let halo = materials.add(theme::SEA);
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    let mut fills: [Option<Handle<ColorMaterial>>; 4] = Default::default();
    for (id, airport) in view.world.airports() {
        let kind = airport.kind();
        let fill = fills[usize::from(kind_rank(kind))]
            .get_or_insert_with(|| materials.add(kind_color(kind)))
            .clone();
        let z = 10.0 + f32::from(kind_rank(kind));
        commands
            .spawn((
                AirportSymbol {
                    id,
                    kind,
                    position: Vec2::ZERO,
                },
                MapAnchor::new(MapProjection::new(airport.location())),
                Mesh2d(disc.clone()),
                MeshMaterial2d(fill),
                Transform::from_xyz(0.0, 0.0, z),
                layer.clone(),
            ))
            .with_child((
                Mesh2d(disc.clone()),
                MeshMaterial2d(halo.clone()),
                Transform::from_xyz(0.0, 0.0, -0.05).with_scale(Vec3::splat(HALO)),
                layer.clone(),
            ));
    }
}

/// Keeps symbols a constant size on screen, applies level of detail and
/// records where each is drawn.
fn update_symbols(
    rig: Option<Res<CameraRig>>,
    mut symbols: Query<(&mut AirportSymbol, &mut Transform, &mut Visibility)>,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    for (mut symbol, mut transform, mut visibility) in &mut symbols {
        let position = transform.translation.truncate();
        if symbol.position != position {
            symbol.position = position;
        }
        // Written only when it changes: rewriting it every frame would
        // propagate every airport's transform every frame.
        let size = Vec3::splat(lod::glyph_radius(symbol.kind) * lod::symbol_factor(scale) * scale);
        if transform.scale != size {
            transform.scale = size;
        }
        visibility.set_if_neq(if lod::airport_visible(symbol.kind, scale) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// A runway as a quad between its threshold positions.
fn runway_quad(runway: &Runway, projection: &MapProjection) -> [Vec2; 4] {
    let [a, b] = runway.ends().map(|end| projection.project(end.threshold()));
    let along = (b - a).normalize_or_zero();
    let stretch = MapProjection::units_per_meter(runway.ends()[0].threshold());
    #[allow(clippy::cast_possible_truncation)]
    let half_width = (runway.width().value() * stretch / 2.0) as f32;
    let across = along.perp() * half_width;
    [a - across, b - across, b + across, a + across]
}

/// Runways are meshed in tiles of this many degrees, each in its own frame
/// (plan M5 D130).
const RUNWAY_TILE: f64 = 10.0;

/// The tile of `point`: south-west corner in whole tiles.
// SAFETY: latitudes and longitudes over RUNWAY_TILE are far inside i32.
#[allow(clippy::cast_possible_truncation)]
fn runway_tile(point: LatLon) -> (i32, i32) {
    (
        (point.latitude().value() / RUNWAY_TILE).floor() as i32,
        (point.longitude().value() / RUNWAY_TILE).floor() as i32,
    )
}

fn spawn_runways(
    mut commands: Commands,
    view: Res<WorldView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // Per tile: its frame, and paved and soft quads.
    let mut tiles: HashMap<(i32, i32), (MapProjection, Quads, Quads)> = HashMap::new();
    for (id, airport) in view.world.airports() {
        let (frame, paved, soft) =
            tiles
                .entry(runway_tile(airport.location()))
                .or_insert_with(|| {
                    (
                        MapProjection::new(airport.location()),
                        Quads::default(),
                        Quads::default(),
                    )
                });
        for runway in view.world.runways_of(id) {
            let quads = if runway.surface() == Surface::Paved {
                &mut *paved
            } else {
                &mut *soft
            };
            quads.push(runway_quad(runway, frame));
        }
    }
    let paved_material = materials.add(theme::RUNWAY_PAVED);
    let soft_material = materials.add(theme::RUNWAY_SOFT);
    let mut tiles: Vec<_> = tiles.into_iter().collect();
    tiles.sort_by_key(|(key, _)| *key);
    for (_, (frame, paved, soft)) in tiles {
        for (quads, material) in [(paved, &paved_material), (soft, &soft_material)] {
            if quads.indices.is_empty() {
                continue;
            }
            commands.spawn((
                RunwayLayer,
                MapAnchor::new(frame),
                Mesh2d(meshes.add(quads.into_mesh())),
                MeshMaterial2d(material.clone()),
                Transform::from_xyz(0.0, 0.0, 5.0),
                Visibility::Hidden,
                RenderLayers::layer(OVERLAY_LAYER),
            ));
        }
    }
}

/// Quads collected into one mesh.
#[derive(Default)]
struct Quads {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Quads {
    #[allow(clippy::cast_possible_truncation)]
    fn push(&mut self, corners: [Vec2; 4]) {
        // SAFETY: a few thousand runways stay far below u32::MAX vertices.
        let base = self.positions.len() as u32;
        self.positions
            .extend(corners.iter().map(|c| [c.x, c.y, 0.0]));
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

fn update_runways(
    rig: Option<Res<CameraRig>>,
    mut layers: Query<&mut Visibility, With<RunwayLayer>>,
) {
    let Some(rig) = rig else {
        return;
    };
    let visible = lod::runways_visible(rig.state.scale());
    for mut visibility in &mut layers {
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

fn draw_selection(
    rig: Option<Res<CameraRig>>,
    selection: Res<Selection>,
    symbols: Query<&AirportSymbol>,
    mut gizmos: Gizmos,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    for symbol in &symbols {
        let radius = lod::glyph_radius(symbol.kind) * lod::symbol_factor(scale) * scale;
        if Some(symbol.id) == selection.selected_airport() {
            gizmos.circle_2d(symbol.position, radius * SELECTED_RING, theme::ACCENT);
            gizmos.circle_2d(
                symbol.position,
                radius * 2.0,
                theme::ACCENT.with_alpha(0.35),
            );
        } else if Some(symbol.id) == selection.hovered_airport() {
            gizmos.circle_2d(symbol.position, radius * 2.0, theme::TEXT.with_alpha(0.6));
        }
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{
        LatLon,
        units::{Degrees, Meters},
        world::{GeometrySource, RunwayDefinition, RunwayEnd},
    };
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_runway_quad_has_true_width_and_length() -> Result<()> {
        let projection = MapProjection::new(aerocosm_core::LatLon::new(51.163_4, 10.447_7)?);
        let start = LatLon::new(50.0, 8.5)?;
        let heading = Degrees::new(90.0)?;
        let end = start.destination(heading, Meters::new(3000.0)?);
        let definition = RunwayDefinition {
            airport: "EDDF".parse()?,
            ends: [
                RunwayEnd::new("09".parse()?, start, heading, Meters::new(0.0)?)?,
                RunwayEnd::new("27".parse()?, end, Degrees::new(270.0)?, Meters::new(0.0)?)?,
            ],
            length: Meters::new(3000.0)?,
            width: Meters::new(45.0)?,
            surface: Surface::Paved,
            geometry: GeometrySource::Surveyed,
        };
        let runway = Runway::new(Id::new(0), &definition)?;
        let [a, b, _, d] = runway_quad(&runway, &projection);
        #[allow(clippy::cast_possible_truncation)]
        let stretch = MapProjection::units_per_meter(start) as f32;
        assert!(
            ((b - a).length() / stretch - 3000.0).abs() < 2.0,
            "length {}",
            (b - a).length() / stretch
        );
        assert!(
            ((d - a).length() / stretch - 45.0).abs() < 0.1,
            "width {}",
            (d - a).length() / stretch
        );
        Ok(())
    }

    #[test]
    fn test_quads_index_each_quad_from_its_own_vertices() {
        let mut quads = Quads::default();
        quads.push([Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
        quads.push([Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
        assert_eq!(quads.positions.len(), 8);
        assert_eq!(&quads.indices[6..], &[4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn test_kind_colors_highlight_major_airports() {
        assert_eq!(kind_color(AirportKind::Major), theme::ACCENT);
        assert_ne!(kind_color(AirportKind::Airfield), theme::ACCENT);
    }
}
