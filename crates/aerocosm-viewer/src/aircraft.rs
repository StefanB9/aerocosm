//! Aircraft on the map: silhouettes by size class, coloured by phase,
//! interpolated between ticks, with trails (D33, D49–D51). The logic lives in
//! `flights` and `silhouette`; these systems gather state and apply it.

use aerocosm_core::{
    Id, SimTime,
    ops::{Airframe, FlightView, holding_pattern},
};
use bevy::{
    app::{App, Plugin, Update},
    asset::{Assets, Handle},
    camera::visibility::{RenderLayers, Visibility},
    color::{Alpha, Color, Srgba},
    ecs::{
        change_detection::DetectChangesMut,
        component::Component,
        entity::Entity,
        hierarchy::ChildOf,
        resource::Resource,
        schedule::{IntoScheduleConfigs, common_conditions::resource_added},
        system::{Commands, Query, Res, ResMut},
    },
    gizmos::gizmos::Gizmos,
    log::error,
    math::{Quat, Rect, Vec2, Vec3},
    mesh::{Mesh, Mesh2d},
    sprite_render::{ColorMaterial, MeshMaterial2d},
    tasks::{ComputeTaskPool, TaskPool},
    transform::components::Transform,
};

use crate::{
    camera::{CameraRig, CameraSet, PointerState},
    data::OVERLAY_LAYER,
    driver::{SimDriver, SimReady, SimSet},
    flights::{
        PhaseBand, Pose, Trail, blend, display_length, flight_visible, in_view, rotation,
        trail_visible,
    },
    pool::Assignment,
    projection::{MapProjection, wrap_near},
    selection::Selection,
    silhouette::{
        HALO_WIDTH, SizeClass, WIDE_HALO_WIDTH, halo, halo_width, outline, pixel_length,
        real_length, size_class,
    },
    tessellate::fill_outline,
    theme,
};

/// Aircraft draw above airports (z 10–13) and runways (z 5).
const AIRCRAFT_Z: f32 = 20.0;
/// The selected or hovered aircraft draws above the others.
const HIGHLIGHT_Z: f32 = 21.0;
/// An aircraft's halo draws just behind it, above everything on the ground.
const HALO_Z: f32 = -0.5;
/// Opacity of holding pattern outlines.
const HOLD_ALPHA: f32 = 0.35;
/// Trail opacity at the aircraft, fading to zero at its oldest point.
const TRAIL_ALPHA: f32 = 0.55;

/// One tail's aircraft symbol; `position` is where it is drawn this frame.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub(crate) struct AircraftSprite {
    pub(crate) airframe: Id<Airframe>,
    pub(crate) position: Vec2,
}

impl AircraftSprite {
    /// The symbol of `tail`, drawn at `position`.
    pub(crate) const fn new(airframe: Id<Airframe>, position: Vec2) -> Self {
        Self { airframe, position }
    }
}

/// The silhouette and current colour band of a sprite, and whether it is
/// small enough on screen for the wide halo.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct SpriteLook {
    class: SizeClass,
    band: PhaseBand,
    small: bool,
}

/// The halo behind a sprite: which class and width its mesh is for.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct Halo {
    class: SizeClass,
    wide: bool,
}

/// What sprites are made of: each airframe's size class, the silhouettes
/// by class and the halos' material.
#[derive(Resource)]
struct SpriteParts {
    classes: Vec<SizeClass>,
    shapes: Vec<Handle<Mesh>>,
    dark: Handle<ColorMaterial>,
}

/// The sprites, one per aircraft shown, reused from frame to frame (plan M9
/// PR 2): slot `i` of `assignment` is sprite `entities[i]`.
#[derive(Resource, Default)]
struct SpritePool {
    entities: Vec<Entity>,
    assignment: Assignment,
}

/// Halo meshes per size class: thin, and wide for aircraft small on screen.
#[derive(Resource)]
struct HaloMeshes {
    thin: Vec<Handle<Mesh>>,
    wide: Vec<Handle<Mesh>>,
}

/// One material per phase band.
#[derive(Resource)]
struct BandMaterials([Handle<ColorMaterial>; 5]);

/// The last two tick states of one tail's flight.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Slot {
    previous: Option<Pose>,
    current: Option<Pose>,
    view: Option<FlightView>,
    trail: Trail,
}

impl Slot {
    /// Takes the airframe's flight at a new tick, sampled at `time`: none if
    /// it is parked. `single_step` says the tick follows the last one
    /// sampled, so the last pose is where the blend starts.
    fn update(&mut self, sampled: Option<&(FlightView, Pose)>, single_step: bool, time: SimTime) {
        let Some(&(view, pose)) = sampled else {
            if self.view.is_some() {
                *self = Self::default();
            }
            return;
        };
        let new_leg = self
            .view
            .is_none_or(|old| old.scheduled_departure() != view.scheduled_departure());
        if new_leg {
            self.trail.clear();
        }
        // Interpolate across exactly one tick; after a jump, start afresh.
        self.previous = if single_step && !new_leg {
            // Across the antimeridian, from the copy next to it.
            self.current
                .map(|previous| Pose {
                    position: wrap_near(previous.position, pose.position),
                    ..previous
                })
                .or(Some(pose))
        } else {
            Some(pose)
        };
        self.current = Some(pose);
        if view.phase().is_airborne() {
            self.trail.push(time, pose.position);
        }
        self.view = Some(view);
    }
}

/// Every tail's flight as last sampled from the simulation.
#[derive(Resource, Debug, Default)]
pub(crate) struct TrafficView {
    slots: Vec<Slot>,
    ticks: Option<u64>,
    /// The projection the positions were taken in.
    frame: Option<MapProjection>,
}

impl TrafficView {
    /// The flight `tail` is on, if it is boarding or moving.
    pub(crate) fn flight(&self, tail: Id<Airframe>) -> Option<&FlightView> {
        self.slots.get(tail.as_usize())?.view.as_ref()
    }

    /// Records the simulation's flights if it has ticked since the last call.
    pub(crate) fn sample(&mut self, driver: &SimDriver, projection: &MapProjection) {
        // The floating origin moved (plan M5 D130): shift what is cached.
        if let Some(frame) = self.frame.replace(*projection)
            && frame != *projection
        {
            let by = projection.offset_to(&frame);
            for slot in &mut self.slots {
                for pose in [&mut slot.previous, &mut slot.current]
                    .into_iter()
                    .flatten()
                {
                    pose.position += by;
                }
                slot.trail.shift(by);
            }
        }
        let simulation = driver.simulation();
        let ticks = simulation.ticks();
        if self.ticks == Some(ticks) && !self.slots.is_empty() {
            return;
        }
        let single_step = self.ticks.is_some_and(|last| ticks == last + 1);
        self.ticks = Some(ticks);
        let airframes = simulation.airframe_count();
        self.slots.resize(airframes, Slot::default());
        // Each airframe's slot depends on that airframe alone, so tasks
        // update their own slots in place (plan M9 PR 2).
        let time = simulation.time();
        let pool = ComputeTaskPool::get_or_init(TaskPool::default);
        pool.scope(|scope| {
            for (part, slots) in self.slots.chunks_mut(SAMPLED_PER_TASK).enumerate() {
                scope.spawn(async move {
                    let start = part * SAMPLED_PER_TASK;
                    for (offset, slot) in slots.iter_mut().enumerate() {
                        let sampled = sample_flight(simulation, projection, start + offset);
                        slot.update(sampled.as_ref(), single_step, time);
                    }
                });
            }
        });
    }

    /// The pose of `tail` a fraction `t` of the way into the next tick.
    #[cfg(test)]
    fn pose(&self, tail: Id<Airframe>, t: f32) -> Option<Pose> {
        let slot = self.slots.get(tail.as_usize())?;
        Some(blend(slot.previous?, slot.current?, t))
    }
}

/// Aircraft sampled per task: enough to outweigh starting a task.
const SAMPLED_PER_TASK: usize = 2048;

/// The flight of the aircraft at `index`, if boarding or moving, with its
/// pose in `projection`.
fn sample_flight(
    simulation: &aerocosm_core::Simulation,
    projection: &MapProjection,
    index: usize,
) -> Option<(FlightView, Pose)> {
    let view = simulation.flight(Id::from_usize(index).ok()?)?;
    let pose = Pose {
        position: projection.project(view.position()),
        // Courses are in [0, 360); f32 is ample for drawing.
        #[allow(clippy::cast_possible_truncation)]
        heading: view.heading().value() as f32,
    };
    Some((view, pose))
}

/// Systems that place the aircraft sprites; readers of their positions run
/// after.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DrawSet;

/// Draws the aircraft.
pub(crate) struct AircraftPlugin;

impl Plugin for AircraftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrafficView>().add_systems(
            Update,
            (
                prepare_sprites.run_if(resource_added::<SimDriver>),
                sample_traffic.after(SimSet).after(CameraSet),
                draw_aircraft
                    .in_set(DrawSet)
                    .after(sample_traffic)
                    .after(CameraSet),
                size_halos.after(DrawSet),
                draw_trails.after(DrawSet),
            )
                .in_set(SimReady),
        );
    }
}

fn prepare_sprites(
    mut commands: Commands,
    driver: Res<SimDriver>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let simulation = driver.simulation();
    let (Some(schedule), Some(traffic)) = (simulation.schedule(), simulation.scenario().traffic())
    else {
        return;
    };
    let mut shapes = Vec::with_capacity(SizeClass::ALL.len());
    let mut halos = HaloMeshes {
        thin: Vec::with_capacity(SizeClass::ALL.len()),
        wide: Vec::with_capacity(SizeClass::ALL.len()),
    };
    for class in SizeClass::ALL {
        let shape = outline(class);
        match (
            fill_outline(&shape),
            fill_outline(&halo(&shape, HALO_WIDTH)),
            fill_outline(&halo(&shape, WIDE_HALO_WIDTH)),
        ) {
            (Ok(triangles), Ok(thin), Ok(wide)) => {
                shapes.push(meshes.add(triangles.into_mesh()));
                halos.thin.push(meshes.add(thin.into_mesh()));
                halos.wide.push(meshes.add(wide.into_mesh()));
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                error!("cannot draw aircraft: {error:#}");
                return;
            }
        }
    }
    let dark = materials.add(theme::AIRCRAFT_HALO);
    let bands = BandMaterials(PhaseBand::ALL.map(|band| materials.add(band.color())));
    // Aircraft are numbered as the core numbers them: the schedule's tails,
    // then its spares (plan M8 D170). Each starts as the airframe of its
    // rotation's index, and swaps only within a pool of one type.
    let classes = schedule
        .tails()
        .map(|(_, info)| info)
        .chain(schedule.spares())
        .map(|info| {
            traffic
                .aircraft()
                .get(info.aircraft())
                .map_or(SizeClass::NarrowBody, size_class)
        })
        .collect();
    commands.insert_resource(SpriteParts {
        classes,
        shapes,
        dark,
    });
    commands.init_resource::<SpritePool>();
    commands.insert_resource(bands);
    commands.insert_resource(halos);
}

/// Gives each halo the mesh for its aircraft's class and size on screen.
fn size_halos(
    meshes: Option<Res<HaloMeshes>>,
    looks: Query<&SpriteLook>,
    mut halos: Query<(&ChildOf, &mut Halo, &mut Mesh2d)>,
) {
    let Some(meshes) = meshes else {
        return;
    };
    for (child_of, mut halo, mut mesh) in &mut halos {
        let Ok(look) = looks.get(child_of.parent()) else {
            continue;
        };
        let wanted = Halo {
            class: look.class,
            wide: look.small,
        };
        if *halo != wanted {
            *halo = wanted;
            let set = if look.small {
                &meshes.wide
            } else {
                &meshes.thin
            };
            mesh.0 = set[look.class.index()].clone();
        }
    }
}

fn sample_traffic(
    driver: Res<SimDriver>,
    projection: Res<MapProjection>,
    mut traffic: ResMut<TrafficView>,
) {
    traffic.sample(&driver, &projection);
}

fn tick_fraction(driver: &SimDriver) -> f32 {
    // A fraction in 0..1; f32 is ample for blending.
    #[allow(clippy::cast_possible_truncation)]
    let t = driver.tick_fraction() as f32;
    t.clamp(0.0, 1.0)
}

/// A new sprite of the pool, hidden until it is given an aircraft.
fn spawn_sprite(
    commands: &mut Commands,
    parts: &SpriteParts,
    halos: &HaloMeshes,
    bands: &BandMaterials,
) -> Entity {
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    let class = SizeClass::NarrowBody;
    commands
        .spawn((
            AircraftSprite::new(Id::new(0), Vec2::ZERO),
            SpriteLook {
                class,
                band: PhaseBand::Ground,
                small: true,
            },
            Mesh2d(parts.shapes[class.index()].clone()),
            MeshMaterial2d(bands.0[PhaseBand::Ground.index()].clone()),
            Transform::from_xyz(0.0, 0.0, AIRCRAFT_Z),
            Visibility::Hidden,
            layer.clone(),
        ))
        .with_children(|sprite| {
            sprite.spawn((
                Halo { class, wide: true },
                Mesh2d(halos.wide[class.index()].clone()),
                MeshMaterial2d(parts.dark.clone()),
                Transform::from_xyz(0.0, 0.0, HALO_Z),
                layer,
            ));
        })
        .id()
}

/// What sprites are drawn with, once the simulation is ready.
type SpriteAssets<'w> = (
    Option<Res<'w, BandMaterials>>,
    Option<Res<'w, SpriteParts>>,
    Option<Res<'w, HaloMeshes>>,
);

/// The pool's sprites, as `draw_aircraft` changes them.
type PooledSprites<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut AircraftSprite,
        &'static mut SpriteLook,
        &'static mut Mesh2d,
        &'static mut Transform,
        &'static mut Visibility,
        &'static mut MeshMaterial2d<ColorMaterial>,
    ),
>;

#[allow(clippy::too_many_arguments)]
fn draw_aircraft(
    mut commands: Commands,
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    driver: Res<SimDriver>,
    traffic: Res<TrafficView>,
    selection: Res<Selection>,
    assets: SpriteAssets,
    pool: Option<ResMut<SpritePool>>,
    mut sprites: PooledSprites,
) {
    let (Some(rig), (Some(bands), Some(parts), Some(halos)), Some(mut pool)) = (rig, assets, pool)
    else {
        return;
    };
    let scale = rig.state.scale();
    let centre = rig.state.center();
    let screen = Rect::from_corners(
        rig.state.screen_to_world(Vec2::ZERO, pointer.viewport),
        rig.state
            .screen_to_world(pointer.viewport, pointer.viewport),
    );
    let symbol_factor = crate::lod::symbol_factor(scale);
    let t = tick_fraction(&driver);
    // The aircraft shown this frame: visible at this zoom and on screen,
    // drawn at the copy nearest the camera (plan M5 D131).
    let shown: Vec<(usize, &FlightView, Pose)> = traffic
        .slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            let view = slot.view.as_ref()?;
            if !flight_visible(view.phase(), scale) {
                return None;
            }
            let pose = blend(slot.previous?, slot.current?, t);
            let position = wrap_near(pose.position, centre);
            in_view(position, screen, scale).then_some((index, view, Pose { position, ..pose }))
        })
        .collect();
    let size = pool
        .assignment
        .assign(shown.iter().map(|(index, _, _)| *index));
    while pool.entities.len() < size {
        let entity = spawn_sprite(&mut commands, &parts, &halos, &bands);
        pool.entities.push(entity);
    }
    let mut by_slot: Vec<Option<(&FlightView, Pose)>> = vec![None; size];
    for &(index, view, pose) in &shown {
        if let Some(slot) = pool.assignment.slot_of(index) {
            by_slot[slot] = Some((view, pose));
        }
    }
    for (slot, &entity) in pool.entities.iter().enumerate() {
        let Ok((mut sprite, mut look, mut mesh, mut transform, mut visibility, mut material)) =
            sprites.get_mut(entity)
        else {
            // Spawned this frame: shown from the next.
            continue;
        };
        let (Some((view, pose)), Some(airframe)) = (
            by_slot.get(slot).copied().flatten(),
            pool.assignment
                .airframe_in(slot)
                .and_then(|index| Id::<Airframe>::from_usize(index).ok()),
        ) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        sprite.airframe = airframe;
        sprite.position = pose.position;
        let class = parts
            .classes
            .get(airframe.as_usize())
            .copied()
            .unwrap_or(SizeClass::NarrowBody);
        if look.class != class {
            look.class = class;
            mesh.0 = parts.shapes[class.index()].clone();
        }
        #[allow(clippy::cast_possible_truncation)]
        let units_per_meter = MapProjection::units_per_meter(view.position()) as f32;
        let highlighted = selection.selected_flight() == Some(airframe)
            || selection.hovered_flight() == Some(airframe);
        let z = if highlighted { HIGHLIGHT_Z } else { AIRCRAFT_Z };
        let length = display_length(
            pixel_length(class) * symbol_factor,
            real_length(class),
            scale,
            units_per_meter,
        );
        look.small = halo_width(length / scale.max(f32::EPSILON)) > HALO_WIDTH;
        // z unscaled: the halo stays just behind at every zoom.
        let placed = Transform::from_translation(pose.position.extend(z))
            .with_rotation(Quat::from_rotation_z(rotation(pose.heading)))
            .with_scale(Vec3::new(length, length, 1.0));
        transform.set_if_neq(placed);
        let band = PhaseBand::of(view.phase());
        if look.band != band {
            look.band = band;
            material.0 = bands.0[band.index()].clone();
        }
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// The airline's colour for the selected flight (D49).
fn airline_color(driver: &SimDriver, view: &FlightView) -> Option<Color> {
    let traffic = driver.simulation().scenario().traffic()?;
    let [r, g, b] = crate::flights::legible(traffic.airlines().get(view.airline())?.colour().rgb());
    Some(Color::Srgba(Srgba::rgb_u8(r, g, b)))
}

fn draw_trails(
    rig: Option<Res<CameraRig>>,
    driver: Res<SimDriver>,
    traffic: Res<TrafficView>,
    selection: Res<Selection>,
    projection: Res<MapProjection>,
    sprites: Query<(&AircraftSprite, &Visibility)>,
    mut gizmos: Gizmos,
) {
    let Some(rig) = rig else {
        return;
    };
    let scale = rig.state.scale();
    for (sprite, visibility) in &sprites {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let selected = selection.selected_flight() == Some(sprite.airframe);
        let hovered = selection.hovered_flight() == Some(sprite.airframe);
        let Some(slot) = traffic.slots.get(sprite.airframe.as_usize()) else {
            continue;
        };
        let Some(view) = slot.view else {
            continue;
        };
        if view.phase().is_airborne() && trail_visible(scale, selected || hovered) {
            let base = if selected {
                airline_color(&driver, &view).unwrap_or(theme::ACCENT)
            } else {
                PhaseBand::of(view.phase()).color()
            };
            let points = slot
                .trail
                .points()
                .map(|(position, age)| {
                    (
                        wrap_near(position, sprite.position),
                        base.with_alpha(TRAIL_ALPHA * (1.0 - age)),
                    )
                })
                .chain(std::iter::once((
                    sprite.position,
                    base.with_alpha(TRAIL_ALPHA),
                )));
            gizmos.linestrip_gradient_2d(points);
        }
        if let Some(hold) = view.holding()
            && trail_visible(scale, selected || hovered)
        {
            let pattern = holding_pattern(hold.fix(), hold.inbound())
                .into_iter()
                .map(|point| wrap_near(projection.project(point), sprite.position));
            gizmos.linestrip_2d(pattern, PhaseBand::Holding.color().with_alpha(HOLD_ALPHA));
        }
        if selected || hovered {
            let radius = pixel_length(SizeClass::VeryLarge) * scale;
            let color = if selected {
                theme::ACCENT
            } else {
                theme::TEXT.with_alpha(0.6)
            };
            gizmos.circle_2d(sprite.position, radius, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{RngSeed, SimConfig, Simulation};
    use anyhow::{Context, Result};

    use super::*;

    fn driver() -> Result<SimDriver> {
        let (scenario, _) = crate::data::load()?;
        SimDriver::new(Simulation::new(
            SimConfig::new("2026-11-05T07:00:00Z".parse()?, RngSeed::new(0)),
            scenario,
        )?)
    }

    #[test]
    fn test_traffic_view_samples_flights_and_interpolates() -> Result<()> {
        let mut driver = driver()?;
        let projection = MapProjection::new(aerocosm_core::LatLon::new(51.163_4, 10.447_7)?);
        let mut traffic = TrafficView::default();
        traffic.sample(&driver, &projection);
        let airborne = traffic
            .slots
            .iter()
            .filter(|slot| slot.view.is_some_and(|view| view.phase().is_airborne()))
            .count();
        assert_eq!(airborne, driver.simulation().airborne());
        // Sampled in parallel, the views are the core's, by airframe.
        let mut listed = 0;
        for view in driver.simulation().flights() {
            assert_eq!(traffic.flight(view.airframe()), Some(&view));
            listed += 1;
        }
        assert_eq!(
            traffic.slots.iter().filter(|s| s.view.is_some()).count(),
            listed
        );
        let (index, slot) = traffic
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.view.is_some_and(|view| view.phase().is_airborne()))
            .context("an airborne flight")?;
        let tail = Id::<Airframe>::from_usize(index)?;
        let first = slot.current.context("pose")?;
        assert!(traffic.flight(tail).is_some());

        driver.run_ticks(1)?;
        traffic.sample(&driver, &projection);
        let start = traffic.pose(tail, 0.0).context("pose")?;
        let end = traffic.pose(tail, 1.0).context("pose")?;
        assert!(
            start.position.distance(first.position) < 1e-3,
            "blends from the last tick"
        );
        assert!(end.position.distance(start.position) > 1.0, "and moves");
        Ok(())
    }

    #[test]
    fn test_traffic_view_resamples_only_after_ticks() -> Result<()> {
        let driver = driver()?;
        let projection = MapProjection::new(aerocosm_core::LatLon::new(51.163_4, 10.447_7)?);
        let mut traffic = TrafficView::default();
        traffic.sample(&driver, &projection);
        let before = traffic.slots.clone();
        traffic.sample(&driver, &projection);
        assert_eq!(traffic.slots, before);
        Ok(())
    }
}
