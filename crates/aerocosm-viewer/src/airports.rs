//! Airport symbols, runways drawn to scale, and hover/selection.

use aerocosm_core::{
    Id,
    world::{Airport, AirportKind, Runway, Surface, World},
};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use crate::{
    camera::{CameraRig, PointerState},
    data::OVERLAY_LAYER,
    lod, picking,
    projection::MapProjection,
    theme,
};

/// Pointer reach for hovering a symbol, in pixels.
const HOVER_REACH: f32 = 12.0;
/// Selection ring radius, relative to the symbol radius.
pub(crate) const SELECTED_RING: f32 = 2.4;
/// Halo around a symbol, relative to its radius, for contrast against lines.
const HALO: f32 = 1.45;

/// The simulated world, as rendered.
#[derive(Resource)]
pub(crate) struct WorldView {
    pub(crate) world: World,
}

/// Which airport the pointer is over and which one is selected.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) hovered: Option<Id<Airport>>,
    pub(crate) selected: Option<Id<Airport>>,
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
                    update_symbols,
                    update_runways,
                    hover_and_select.after(crate::camera::CameraSet),
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
    projection: Res<MapProjection>,
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
        let position = projection.project(airport.location());
        let z = 10.0 + f32::from(kind_rank(kind));
        commands
            .spawn((
                AirportSymbol { id, kind, position },
                Mesh2d(disc.clone()),
                MeshMaterial2d(fill),
                Transform::from_translation(position.extend(z)),
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

/// Keeps symbols a constant size on screen and applies level of detail.
fn update_symbols(
    rig: Option<Res<CameraRig>>,
    mut symbols: Query<(&AirportSymbol, &mut Transform, &mut Visibility)>,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    for (symbol, mut transform, mut visibility) in &mut symbols {
        transform.scale = Vec3::splat(lod::glyph_radius(symbol.kind) * scale);
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

fn spawn_runways(
    mut commands: Commands,
    view: Res<WorldView>,
    projection: Res<MapProjection>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mut paved = Quads::default();
    let mut soft = Quads::default();
    for (id, _) in view.world.airports() {
        for runway in view.world.runways_of(id) {
            let quads = if runway.surface() == Surface::Paved {
                &mut paved
            } else {
                &mut soft
            };
            quads.push(runway_quad(runway, &projection));
        }
    }
    for (quads, color) in [(paved, theme::RUNWAY_PAVED), (soft, theme::RUNWAY_SOFT)] {
        commands.spawn((
            RunwayLayer,
            Mesh2d(meshes.add(quads.into_mesh())),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(0.0, 0.0, 5.0),
            Visibility::Hidden,
            RenderLayers::layer(OVERLAY_LAYER),
        ));
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

fn hover_and_select(
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    symbols: Query<(&AirportSymbol, &Visibility)>,
    mut selection: ResMut<Selection>,
) {
    let Some(rig) = rig else {
        return;
    };
    let hovered = match pointer.cursor {
        Some(cursor) if !pointer.over_ui => picking::nearest(
            symbols
                .iter()
                .filter(|(_, visibility)| **visibility != Visibility::Hidden)
                .map(|(symbol, _)| {
                    (
                        symbol.id,
                        rig.state.world_to_screen(symbol.position, pointer.viewport),
                    )
                }),
            cursor,
            HOVER_REACH,
        ),
        _ => None,
    };
    if selection.hovered != hovered {
        selection.hovered = hovered;
    }
    if pointer.clicked {
        selection.selected = hovered;
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
        let radius = lod::glyph_radius(symbol.kind) * scale;
        if Some(symbol.id) == selection.selected {
            gizmos.circle_2d(symbol.position, radius * SELECTED_RING, theme::ACCENT);
            gizmos.circle_2d(
                symbol.position,
                radius * 2.0,
                theme::ACCENT.with_alpha(0.35),
            );
        } else if Some(symbol.id) == selection.hovered {
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
        let projection = MapProjection::germany()?;
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

    fn selection_app(cursor: Option<Vec2>, clicked: bool) -> App {
        use crate::camera::{
            CameraRig,
            controller::{CameraLimits, CameraState},
        };
        let mut app = App::new();
        app.insert_resource(CameraRig::new(
            CameraState::new(Vec2::ZERO, 10.0),
            CameraLimits {
                bounds: Rect::new(-1e6, -1e6, 1e6, 1e6),
                min_scale: 1.0,
                max_scale: 100.0,
            },
        ))
        .insert_resource(PointerState {
            cursor,
            clicked,
            over_ui: false,
            viewport: Vec2::new(800.0, 600.0),
        })
        .init_resource::<Selection>()
        .add_systems(Update, hover_and_select);
        // At scale 10 the screen centre (400, 300) is world (0, 0); this symbol
        // is 50 px right.
        app.world_mut().spawn((
            AirportSymbol {
                id: Id::new(7),
                kind: AirportKind::Major,
                position: Vec2::new(500.0, 0.0),
            },
            Visibility::Inherited,
        ));
        app
    }

    #[test]
    fn test_hover_and_click_select_symbol_under_cursor() {
        let mut app = selection_app(Some(Vec2::new(452.0, 301.0)), true);
        app.update();
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.hovered, Some(Id::new(7)));
        assert_eq!(selection.selected, Some(Id::new(7)));
    }

    #[test]
    fn test_click_on_empty_map_clears_selection() {
        let mut app = selection_app(Some(Vec2::new(100.0, 100.0)), true);
        app.world_mut().resource_mut::<Selection>().selected = Some(Id::new(7));
        app.update();
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.hovered, None);
        assert_eq!(selection.selected, None);
    }

    #[test]
    fn test_hover_without_click_keeps_selection() {
        let mut app = selection_app(Some(Vec2::new(452.0, 301.0)), false);
        app.update();
        assert_eq!(app.world().resource::<Selection>().selected, None);
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
