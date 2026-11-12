//! Map labels: UI text positioned over the map each frame, placed without
//! collisions.

use aerocosm_core::{
    Id,
    ground::GroundNetwork,
    world::{Airport, AirportKind},
};
use bevy::{
    picking::Pickable,
    prelude::*,
    ui::{ComputedNode, UiGlobalTransform},
};

use crate::{
    airports::WorldView,
    camera::{CameraRig, PointerState},
    data::{Fonts, MapData},
    ground::{
        GroundView,
        geometry::{stand_names_visible, stretch_anchors, taxiway_names_visible},
        named_edges,
    },
    labels::{LabelCandidate, place_labels},
    lod,
    projection::MapProjection,
    selection::Selection,
    theme,
    ui::MapCover,
};

/// What a label belongs to, for level of detail.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Subject {
    Airport(Id<Airport>, AirportKind),
    Place(u8),
    Runway,
    Taxiway,
    Stand,
}

/// A label anchored to a map position.
#[derive(Component)]
struct MapLabel {
    subject: Subject,
    position: Vec2,
    /// Direction (map units, y up) in which the label is pushed away from its
    /// anchor; runway designators sit just beyond their threshold.
    outward: Vec2,
}

/// How far runway designators sit beyond their threshold, in pixels.
const DESIGNATOR_OFFSET: f32 = 14.0;
/// Extra space between a selected airport's ring and its label, in pixels.
const SELECTED_GAP: f32 = 6.0;

/// Label entities in placement priority order.
#[derive(Resource, Default)]
struct LabelOrder(Vec<Entity>);

/// Spawns and places map labels.
pub(crate) struct LabelsPlugin;

impl Plugin for LabelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LabelOrder>()
            .add_systems(Startup, spawn_labels.after(crate::data::setup_fonts))
            .add_systems(Update, place.after(crate::camera::CameraSet));
    }
}

/// Placement tier: lower is placed first and wins collisions.
fn tier(subject: Subject) -> u8 {
    match subject {
        Subject::Airport(_, AirportKind::Major | AirportKind::Edge) => 0,
        Subject::Place(rank) if rank <= 3 => 1,
        Subject::Airport(_, AirportKind::Regional) => 2,
        Subject::Place(_) => 3,
        Subject::Airport(_, AirportKind::Airfield) => 4,
        Subject::Runway => 5,
        Subject::Taxiway => 6,
        Subject::Stand => 7,
    }
}

fn spawn_labels(
    mut commands: Commands,
    fonts: Res<Fonts>,
    view: Res<WorldView>,
    map: Res<MapData>,
    ground: Res<GroundView>,
    projection: Res<MapProjection>,
    mut order: ResMut<LabelOrder>,
) {
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    let shadow = TextShadow {
        offset: Vec2::new(0.0, 1.0),
        color: theme::SEA.with_alpha(0.9),
    };
    let mut labels: Vec<(u8, usize, Entity)> =
        Vec::with_capacity(view.world.airport_count() + map.basemap.places().len());
    let mut spawn = |commands: &mut Commands,
                     subject: Subject,
                     (position, outward): (Vec2, Vec2),
                     text: String| {
        let (font, color) = style(subject, &fonts);
        let entity = commands
            .spawn((
                MapLabel {
                    subject,
                    position,
                    outward,
                },
                Text::new(text),
                font,
                TextColor(color),
                shadow,
                // Hidden with `Visibility`, not `Display::None`, so the text is
                // still laid out and its measured size is available for
                // placement.
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                Visibility::Hidden,
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id();
        labels.push((tier(subject), labels.len(), entity));
    };
    for (id, airport) in view.world.airports() {
        let code = airport
            .iata()
            .map_or_else(|| airport.ident().to_string(), |iata| iata.to_string());
        spawn(
            &mut commands,
            Subject::Airport(id, airport.kind()),
            (projection.project(airport.location()), Vec2::ZERO),
            code,
        );
    }
    // Places arrive sorted by importance.
    for place in map.basemap.places() {
        spawn(
            &mut commands,
            Subject::Place(place.rank()),
            (projection.project(place.location()), Vec2::ZERO),
            place.name().to_owned(),
        );
    }
    for (id, _) in view.world.airports() {
        for runway in view.world.runways_of(id) {
            let [a, b] = runway.ends().map(|end| projection.project(end.threshold()));
            for (end, here, other) in [(runway.ends()[0], a, b), (runway.ends()[1], b, a)] {
                spawn(
                    &mut commands,
                    Subject::Runway,
                    (here, (here - other).normalize_or_zero()),
                    end.designator().to_string(),
                );
            }
        }
    }
    for (subject, position, text) in ground_labels(&ground.ground, &projection) {
        spawn(&mut commands, subject, (position, Vec2::ZERO), text);
    }
    labels.sort_by_key(|(tier, index, _)| (*tier, *index));
    order.0 = labels.into_iter().map(|(_, _, entity)| entity).collect();
}

/// The typeface, size and colour of `subject`'s label.
fn style(subject: Subject, fonts: &Fonts) -> (TextFont, Color) {
    let (size, semibold, color) = match subject {
        Subject::Airport(_, AirportKind::Major | AirportKind::Edge) => (13.0, true, theme::TEXT),
        Subject::Airport(_, AirportKind::Regional) => (12.0, true, theme::REGIONAL.with_alpha(0.9)),
        Subject::Airport(_, AirportKind::Airfield) => (11.0, false, theme::TEXT_MUTED),
        Subject::Place(_) => (12.0, false, theme::TEXT_PLACE),
        Subject::Runway => (10.0, true, theme::TEXT_MUTED),
        Subject::Taxiway => (10.0, true, theme::TAXIWAY.with_alpha(0.9)),
        Subject::Stand => (9.0, false, theme::TEXT_MUTED),
    };
    let font = if semibold {
        fonts.semibold.clone()
    } else {
        fonts.regular.clone()
    };
    (TextFont::from_font_size(size).with_font(font), color)
}

/// Taxiway names, one per named stretch, and stand names at every mapped
/// airport (plan M4 D87).
fn ground_labels(
    ground: &GroundNetwork,
    projection: &MapProjection,
) -> Vec<(Subject, Vec2, String)> {
    let mut labels = Vec::new();
    for layout in ground.airports() {
        labels.extend(
            stretch_anchors(&named_edges(layout, projection))
                .into_iter()
                .map(|(name, at)| (Subject::Taxiway, at, name.to_string())),
        );
        labels.extend(layout.stands().iter().map(|stand| {
            let position = layout.nodes()[stand.node().as_usize()].position();
            (
                Subject::Stand,
                projection.project(position),
                stand.name().to_string(),
            )
        }));
    }
    labels
}

/// Per-frame scratch buffers, reused to avoid allocating every frame.
#[derive(Default)]
struct Scratch {
    candidates: Vec<LabelCandidate>,
    owners: Vec<Entity>,
    placed: Vec<(usize, Rect)>,
    shown: Vec<(Entity, Vec2)>,
    blocked: Vec<Rect>,
}

fn place(
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    selection: Res<Selection>,
    order: Res<LabelOrder>,
    mut labels: Query<(&MapLabel, &mut Node, &mut Visibility, &ComputedNode)>,
    covers: Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility), With<MapCover>>,
    mut scratch: Local<Scratch>,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    let scratch = &mut *scratch;
    scratch.candidates.clear();
    scratch.owners.clear();

    // The selected airport's label goes first so it is always shown.
    let selected = order.0.iter().copied().find(|entity| {
        labels.get(*entity).is_ok_and(|(label, _, _, _)| {
            matches!(label.subject, Subject::Airport(id, _) if Some(id) == selection.selected_airport())
        })
    });
    let others = order
        .0
        .iter()
        .copied()
        .filter(|entity| Some(*entity) != selected);
    for entity in selected.into_iter().chain(others) {
        let Ok((label, _, _, computed)) = labels.get(entity) else {
            continue;
        };
        let (visible, gap) = match label.subject {
            Subject::Airport(id, kind) if Some(id) == selection.selected_airport() => (
                true,
                Some(lod::glyph_radius(kind) * crate::airports::SELECTED_RING + SELECTED_GAP),
            ),
            Subject::Airport(_, kind) => (
                lod::airport_label_visible(kind, scale),
                Some(lod::glyph_radius(kind) + 5.0),
            ),
            Subject::Place(rank) => (lod::place_label_visible(rank, scale), None),
            Subject::Runway => (lod::runways_visible(scale), None),
            Subject::Taxiway => (taxiway_names_visible(scale), None),
            Subject::Stand => (stand_names_visible(scale), None),
        };
        if !visible {
            continue;
        }
        let size = computed.size() * computed.inverse_scale_factor();
        // Before the first layout the size is unknown; estimate a typical
        // label.
        let size = if size.x > 0.0 {
            size
        } else {
            Vec2::new(60.0, 16.0)
        };
        let anchor = rig.state.world_to_screen(label.position, pointer.viewport)
            + Vec2::new(label.outward.x, -label.outward.y) * DESIGNATOR_OFFSET;
        scratch
            .candidates
            .push(LabelCandidate { anchor, size, gap });
        scratch.owners.push(entity);
    }
    scratch.blocked.clear();
    scratch
        .blocked
        .extend(covers.iter().filter(|(_, _, visible)| visible.get()).map(
            |(node, transform, _)| {
                let scale = node.inverse_scale_factor();
                Rect::from_center_size(transform.translation * scale, node.size() * scale)
            },
        ));
    place_labels(
        &scratch.candidates,
        pointer.viewport,
        &scratch.blocked,
        &mut scratch.placed,
    );

    scratch.shown.clear();
    scratch.shown.extend(
        scratch
            .placed
            .iter()
            .map(|(index, rect)| (scratch.owners[*index], rect.min)),
    );
    scratch.shown.sort_unstable_by_key(|(entity, _)| *entity);
    for entity in &order.0 {
        let Ok((_, mut node, mut visibility, _)) = labels.get_mut(*entity) else {
            continue;
        };
        if let Ok(found) = scratch.shown.binary_search_by_key(entity, |(e, _)| *e) {
            let corner = scratch.shown[found].1;
            visibility.set_if_neq(Visibility::Inherited);
            node.left = px(corner.x);
            node.top = px(corner.y);
        } else {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiers_put_major_airports_before_cities_before_airfields() {
        let major = tier(Subject::Airport(Id::new(0), AirportKind::Major));
        let capital = tier(Subject::Place(1));
        let village = tier(Subject::Place(9));
        let airfield = tier(Subject::Airport(Id::new(1), AirportKind::Airfield));
        let runway = tier(Subject::Runway);
        assert!(major < capital && capital < village && village < airfield && airfield < runway);
        // Ground names come last, stands after taxiways.
        assert!(runway < tier(Subject::Taxiway) && tier(Subject::Taxiway) < tier(Subject::Stand));
    }
}
