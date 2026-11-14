//! Map labels: UI text positioned over the map each frame, placed without
//! collisions.
//!
//! The world has about 100,000 things to label (airports, places, runway
//! ends, then taxiways and stands as grounds are built), so labels are kept
//! as plain [`LabelSource`]s and only those placed get a text entity (plan
//! M5 D132): memory and per-frame work follow what is on screen, not the
//! world. An entity not placed for a while is despawned.

use std::collections::HashMap;

use aerocosm_core::{
    Id, LatLon,
    ground::AirportGround,
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
        geometry::{stand_names_visible, stretch_anchors, taxiway_names_visible},
        named_edges,
    },
    labels::{LabelCandidate, place_labels},
    lod,
    projection::{MapProjection, wrap_near},
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

/// Something that may be labelled.
#[derive(Debug, Clone)]
struct LabelSource {
    subject: Subject,
    /// Where it is anchored…
    location: LatLon,
    /// …projected with the current origin (plan M5 D130).
    position: Vec2,
    /// Direction (map units, y up) in which the label is pushed away from its
    /// anchor; runway designators sit just beyond their threshold.
    outward: Vec2,
    text: String,
}

/// Which source a label entity shows: one of the fixed sources, or one of a
/// built ground's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LabelKey {
    Fixed(usize),
    Ground(usize, usize),
}

/// Every label source: airports, places and runway ends in placement
/// priority order, and the names of the grounds built now, by ground.
#[derive(Resource, Default)]
pub(crate) struct LabelSources {
    fixed: Vec<LabelSource>,
    ground: HashMap<usize, Vec<LabelSource>>,
    /// The projection the positions were taken in.
    frame: Option<MapProjection>,
}

impl LabelSources {
    /// Adds the taxiway and stand names of ground `index`, built in `frame`.
    pub(crate) fn add_ground(
        &mut self,
        index: usize,
        layout: &AirportGround,
        frame: &MapProjection,
        projection: &MapProjection,
    ) {
        let sources = ground_labels(layout, frame)
            .into_iter()
            .map(|(subject, location, text)| LabelSource {
                subject,
                location,
                position: projection.project(location),
                outward: Vec2::ZERO,
                text,
            })
            .collect();
        self.ground.insert(index, sources);
    }

    /// Forgets the names of ground `index`.
    pub(crate) fn remove_ground(&mut self, index: usize) {
        self.ground.remove(&index);
    }

    fn get(&self, key: LabelKey) -> Option<&LabelSource> {
        match key {
            LabelKey::Fixed(i) => self.fixed.get(i),
            LabelKey::Ground(ground, i) => self.ground.get(&ground)?.get(i),
        }
    }

    /// Every source in placement order: the fixed ones, then grounds' names
    /// in a stable order.
    fn in_order(&self) -> impl Iterator<Item = (LabelKey, &LabelSource)> {
        let mut grounds: Vec<_> = self.ground.iter().collect();
        grounds.sort_by_key(|(index, _)| **index);
        let fixed = self
            .fixed
            .iter()
            .enumerate()
            .map(|(i, source)| (LabelKey::Fixed(i), source));
        let ground = grounds.into_iter().flat_map(|(index, sources)| {
            sources
                .iter()
                .enumerate()
                .map(move |(i, source)| (LabelKey::Ground(*index, i), source))
        });
        fixed.chain(ground)
    }
}

/// The text entities of placed labels, and the frame each was last placed.
#[derive(Default)]
struct LabelPool {
    entities: HashMap<LabelKey, (Entity, u64)>,
}

impl LabelPool {
    /// Unplaced this many frames, a label's entity is despawned.
    const KEEP_FRAMES: u64 = 120;

    /// Entities to despawn at `frame`: unplaced for too long, or showing a
    /// source that `exists` no longer.
    fn retire(&mut self, frame: u64, exists: impl Fn(LabelKey) -> bool) -> Vec<Entity> {
        let mut retired = Vec::new();
        self.entities.retain(|key, (entity, placed)| {
            let keep = exists(*key) && frame.saturating_sub(*placed) <= Self::KEEP_FRAMES;
            if !keep {
                retired.push(*entity);
            }
            keep
        });
        retired
    }
}

/// A label's text entity.
#[derive(Component)]
struct MapLabel;

/// How far runway designators sit beyond their threshold, in pixels.
const DESIGNATOR_OFFSET: f32 = 14.0;
/// Anchors this far outside the window, in pixels, are still candidates
/// (their labels may reach in).
const OFF_SCREEN: f32 = 120.0;
/// Extra space between a selected airport's ring and its label, in pixels.
const SELECTED_GAP: f32 = 6.0;

/// The UI node all map labels hang from.
#[derive(Resource)]
struct LabelRoot(Entity);

/// Collects and places map labels.
pub(crate) struct LabelsPlugin;

impl Plugin for LabelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LabelSources>()
            .add_systems(Startup, collect_labels)
            .add_systems(
                Update,
                (reproject, place).chain().after(crate::camera::CameraSet),
            );
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

/// The text size of `subject`'s label and whether it is semibold.
fn font(subject: Subject) -> (f32, bool) {
    match subject {
        Subject::Airport(_, AirportKind::Major | AirportKind::Edge) => (13.0, true),
        Subject::Airport(_, AirportKind::Regional) => (12.0, true),
        Subject::Place(_) => (12.0, false),
        Subject::Airport(_, AirportKind::Airfield) => (11.0, false),
        Subject::Runway | Subject::Taxiway => (10.0, true),
        Subject::Stand => (9.0, false),
    }
}

/// The typeface, size and colour of `subject`'s label.
fn style(subject: Subject, fonts: &Fonts) -> (TextFont, Color) {
    let color = match subject {
        Subject::Airport(_, AirportKind::Major | AirportKind::Edge) => theme::TEXT,
        Subject::Airport(_, AirportKind::Regional) => theme::REGIONAL.with_alpha(0.9),
        Subject::Airport(_, AirportKind::Airfield) | Subject::Runway | Subject::Stand => {
            theme::TEXT_MUTED
        }
        Subject::Place(_) => theme::TEXT_PLACE,
        Subject::Taxiway => theme::TAXIWAY.with_alpha(0.9),
    };
    let (size, semibold) = font(subject);
    let face = if semibold {
        fonts.semibold.clone()
    } else {
        fonts.regular.clone()
    };
    (TextFont::from_font_size(size).with_font(face), color)
}

/// The size of `text` before it has been laid out, in pixels: Inter's
/// average advance is a little over half the font size.
fn estimated_size(text: &str, font_size: f32) -> Vec2 {
    // Label texts are short; their length fits f32 exactly.
    #[allow(clippy::cast_precision_loss)]
    let chars = text.chars().count() as f32;
    Vec2::new(chars * font_size * 0.6, font_size * 1.25)
}

fn collect_labels(
    mut commands: Commands,
    view: Res<WorldView>,
    map: Res<MapData>,
    projection: Res<MapProjection>,
    mut sources: ResMut<LabelSources>,
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
    commands.insert_resource(LabelRoot(root));
    let mut fixed: Vec<(u8, usize, LabelSource)> = Vec::new();
    let mut add = |subject: Subject, location: LatLon, outward: Vec2, text: String| {
        let source = LabelSource {
            subject,
            location,
            position: projection.project(location),
            outward,
            text,
        };
        fixed.push((tier(subject), fixed.len(), source));
    };
    for (id, airport) in view.world.airports() {
        let code = airport
            .iata()
            .map_or_else(|| airport.ident().to_string(), |iata| iata.to_string());
        add(
            Subject::Airport(id, airport.kind()),
            airport.location(),
            Vec2::ZERO,
            code,
        );
    }
    // Places arrive sorted by importance.
    for place in map.basemap.places() {
        add(
            Subject::Place(place.rank()),
            place.location(),
            Vec2::ZERO,
            place.name().to_owned(),
        );
    }
    for (id, _) in view.world.airports() {
        for runway in view.world.runways_of(id) {
            let frame = MapProjection::new(runway.ends()[0].threshold());
            let [a, b] = runway.ends().map(|end| frame.project(end.threshold()));
            for (end, here, other) in [(runway.ends()[0], a, b), (runway.ends()[1], b, a)] {
                add(
                    Subject::Runway,
                    end.threshold(),
                    (here - other).normalize_or_zero(),
                    end.designator().to_string(),
                );
            }
        }
    }
    fixed.sort_by_key(|(tier, index, _)| (*tier, *index));
    sources.fixed = fixed.into_iter().map(|(_, _, source)| source).collect();
    sources.frame = Some(*projection);
}

/// Taxiway names, one per named stretch, then stand names of one airport
/// (plan M4 D87), anchors found in its `frame`.
fn ground_labels(layout: &AirportGround, frame: &MapProjection) -> Vec<(Subject, LatLon, String)> {
    let mut labels: Vec<(Subject, LatLon, String)> = stretch_anchors(&named_edges(layout, frame))
        .into_iter()
        .filter_map(|(name, at)| {
            Some((
                Subject::Taxiway,
                frame.unproject(at).ok()?,
                name.to_string(),
            ))
        })
        .collect();
    labels.extend(layout.stands().iter().map(|stand| {
        let position = layout.nodes()[stand.node().as_usize()].position();
        (Subject::Stand, position, stand.name().to_string())
    }));
    labels
}

/// Projects every source again once the floating origin has moved.
fn reproject(projection: Res<MapProjection>, mut sources: ResMut<LabelSources>) {
    if sources.frame == Some(*projection) {
        return;
    }
    let sources = &mut *sources;
    sources.frame = Some(*projection);
    for source in sources
        .fixed
        .iter_mut()
        .chain(sources.ground.values_mut().flatten())
    {
        source.position = projection.project(source.location);
    }
}

/// Per-frame scratch buffers, reused to avoid allocating every frame.
#[derive(Default)]
struct Scratch {
    candidates: Vec<LabelCandidate>,
    owners: Vec<LabelKey>,
    placed: Vec<(usize, Rect)>,
    blocked: Vec<Rect>,
    frame: u64,
}

fn place(
    mut commands: Commands,
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    selection: Res<Selection>,
    sources: Res<LabelSources>,
    fonts: Res<Fonts>,
    root: Option<Res<LabelRoot>>,
    mut labels: Query<(&mut Node, &mut Visibility, &ComputedNode), With<MapLabel>>,
    covers: Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility), With<MapCover>>,
    mut pool: Local<LabelPool>,
    mut scratch: Local<Scratch>,
) {
    let (Some(rig), Some(root)) = (rig, root) else {
        return;
    };
    let scale = rig.state.scale();
    let centre = rig.state.center();
    let screen = Rect::from_corners(Vec2::ZERO, pointer.viewport).inflate(OFF_SCREEN);
    let scratch = &mut *scratch;
    scratch.frame += 1;
    scratch.candidates.clear();
    scratch.owners.clear();

    let selected_airport = selection.selected_airport();
    let is_selected = |source: &LabelSource| matches!(source.subject, Subject::Airport(id, _) if Some(id) == selected_airport);
    // The selected airport's label goes first so it is always shown.
    let selected = sources.in_order().find(|(_, source)| is_selected(source));
    let others = sources
        .in_order()
        .filter(|(_, source)| !is_selected(source));
    for (key, source) in selected.into_iter().chain(others) {
        let (visible, gap) = match source.subject {
            Subject::Airport(id, kind) if Some(id) == selected_airport => (
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
        // At the copy nearest the camera (plan M5 D131); off-screen anchors
        // are dropped before placement.
        let position = wrap_near(source.position, centre);
        let anchor = rig.state.world_to_screen(position, pointer.viewport)
            + Vec2::new(source.outward.x, -source.outward.y) * DESIGNATOR_OFFSET;
        if !screen.contains(anchor) {
            continue;
        }
        let laid_out = pool
            .entities
            .get(&key)
            .and_then(|(entity, _)| labels.get(*entity).ok())
            .map(|(_, _, computed)| computed.size() * computed.inverse_scale_factor())
            .filter(|size| size.x > 0.0);
        let size = laid_out.unwrap_or_else(|| estimated_size(&source.text, font(source.subject).0));
        scratch
            .candidates
            .push(LabelCandidate { anchor, size, gap });
        scratch.owners.push(key);
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

    show_placed(
        &mut commands,
        &mut pool,
        &mut labels,
        &sources,
        &fonts,
        root.0,
        scratch,
    );
}

/// Shows the placed labels, spawning their entities as needed; hides the
/// rest and retires those long unplaced.
fn show_placed(
    commands: &mut Commands,
    pool: &mut LabelPool,
    labels: &mut Query<(&mut Node, &mut Visibility, &ComputedNode), With<MapLabel>>,
    sources: &LabelSources,
    fonts: &Fonts,
    root: Entity,
    scratch: &Scratch,
) {
    let frame = scratch.frame;
    for (index, rect) in &scratch.placed {
        let key = scratch.owners[*index];
        let Some(source) = sources.get(key) else {
            continue;
        };
        if let Some((entity, placed)) = pool.entities.get_mut(&key) {
            *placed = frame;
            if let Ok((mut node, mut visibility, _)) = labels.get_mut(*entity) {
                visibility.set_if_neq(Visibility::Inherited);
                node.left = px(rect.min.x);
                node.top = px(rect.min.y);
            }
        } else {
            let (font, color) = style(source.subject, fonts);
            let entity = commands
                .spawn((
                    MapLabel,
                    Text::new(source.text.clone()),
                    font,
                    TextColor(color),
                    TextShadow {
                        offset: Vec2::new(0.0, 1.0),
                        color: theme::SEA.with_alpha(0.9),
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(rect.min.x),
                        top: px(rect.min.y),
                        ..default()
                    },
                    Pickable::IGNORE,
                    ChildOf(root),
                ))
                .id();
            pool.entities.insert(key, (entity, frame));
        }
    }
    for (entity, placed) in pool.entities.values() {
        if *placed != frame
            && let Ok((_, mut visibility, _)) = labels.get_mut(*entity)
        {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
    for entity in pool.retire(frame, |key| sources.get(key).is_some()) {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

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

    #[test]
    fn test_estimated_size_grows_with_text_and_font() {
        let short = estimated_size("FRA", 13.0);
        let long = estimated_size("Frankfurt", 13.0);
        assert!(long.x > short.x && (long.y - short.y).abs() < f32::EPSILON);
        assert!(estimated_size("FRA", 9.0).x < short.x);
        assert!(short.x > 15.0 && short.x < 40.0, "{short:?}");
    }

    #[test]
    fn test_pool_retires_labels_long_unplaced_or_gone() {
        let mut world = World::new();
        let [a, b, c] = [(); 3].map(|()| world.spawn_empty().id());
        let mut pool = LabelPool::default();
        pool.entities.insert(LabelKey::Fixed(0), (a, 200));
        pool.entities
            .insert(LabelKey::Fixed(1), (b, 200 - LabelPool::KEEP_FRAMES - 1));
        pool.entities.insert(LabelKey::Ground(7, 0), (c, 200));
        let retired = pool.retire(200, |key| key != LabelKey::Ground(7, 0));
        assert_eq!(retired.len(), 2);
        assert!(retired.contains(&b) && retired.contains(&c));
        assert_eq!(pool.entities.len(), 1);
        assert!(pool.entities.contains_key(&LabelKey::Fixed(0)));
    }

    #[test]
    fn test_sources_place_ground_names_after_fixed_ones() -> Result<()> {
        let mut sources = LabelSources::default();
        let at = LatLon::new(50.0, 8.5)?;
        let source = |text: &str| LabelSource {
            subject: Subject::Stand,
            location: at,
            position: Vec2::ZERO,
            outward: Vec2::ZERO,
            text: text.to_owned(),
        };
        sources.fixed.push(source("FRA"));
        sources.ground.insert(3, vec![source("A1"), source("A2")]);
        sources.ground.insert(1, vec![source("V1")]);
        let order: Vec<&str> = sources.in_order().map(|(_, s)| s.text.as_str()).collect();
        assert_eq!(order, ["FRA", "V1", "A1", "A2"]);
        sources.remove_ground(3);
        assert!(sources.get(LabelKey::Ground(3, 0)).is_none());
        assert!(sources.get(LabelKey::Ground(1, 0)).is_some());
        Ok(())
    }
}
