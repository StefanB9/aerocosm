//! Aircraft on the map: silhouettes by size class, coloured by phase,
//! interpolated between ticks, with trails (D33, D49–D51). The logic lives in
//! `flights` and `silhouette`; these systems gather state and apply it.

use aerocosm_core::{Id, ops::FlightView, schedule::Tail};
use bevy::{
    app::{App, Plugin, Startup, Update},
    asset::{Assets, Handle},
    camera::visibility::{RenderLayers, Visibility},
    color::{Alpha, Color, Srgba},
    ecs::{
        change_detection::DetectChangesMut,
        component::Component,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Query, Res, ResMut},
    },
    gizmos::gizmos::Gizmos,
    log::error,
    math::{Quat, Vec2, Vec3},
    mesh::{Mesh, Mesh2d},
    sprite_render::{ColorMaterial, MeshMaterial2d},
    transform::components::Transform,
};

use crate::{
    camera::{CameraRig, CameraSet},
    data::OVERLAY_LAYER,
    driver::{SimDriver, SimSet},
    flights::{
        PhaseBand, Pose, Trail, blend, display_length, flight_visible, rotation, trail_visible,
    },
    projection::MapProjection,
    selection::Selection,
    silhouette::{SizeClass, outline, pixel_length, real_length, size_class},
    tessellate::fill_outline,
    theme,
};

/// Aircraft draw above airports (z 10–13) and runways (z 5).
const AIRCRAFT_Z: f32 = 20.0;
/// The selected or hovered aircraft draws above the others.
const HIGHLIGHT_Z: f32 = 21.0;
/// Trail opacity at the aircraft, fading to zero at its oldest point.
const TRAIL_ALPHA: f32 = 0.55;

/// One tail's aircraft symbol; `position` is where it is drawn this frame.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub(crate) struct AircraftSprite {
    pub(crate) tail: Id<Tail>,
    pub(crate) position: Vec2,
}

impl AircraftSprite {
    /// The symbol of `tail`, drawn at `position`.
    pub(crate) const fn new(tail: Id<Tail>, position: Vec2) -> Self {
        Self { tail, position }
    }
}

/// The silhouette and current colour band of a sprite.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct SpriteLook {
    class: SizeClass,
    band: PhaseBand,
}

/// One material per phase band.
#[derive(Resource)]
struct BandMaterials([Handle<ColorMaterial>; 4]);

/// The last two tick states of one tail's flight.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Slot {
    previous: Option<Pose>,
    current: Option<Pose>,
    view: Option<FlightView>,
    trail: Trail,
}

/// Every tail's flight as last sampled from the simulation.
#[derive(Resource, Debug, Default)]
pub(crate) struct TrafficView {
    slots: Vec<Slot>,
    ticks: Option<u64>,
}

impl TrafficView {
    /// The flight `tail` is on, if it is boarding or moving.
    pub(crate) fn flight(&self, tail: Id<Tail>) -> Option<&FlightView> {
        self.slots.get(tail.as_usize())?.view.as_ref()
    }

    /// Records the simulation's flights if it has ticked since the last call.
    pub(crate) fn sample(&mut self, driver: &SimDriver, projection: &MapProjection) {
        let simulation = driver.simulation();
        let ticks = simulation.ticks();
        if self.ticks == Some(ticks) && !self.slots.is_empty() {
            return;
        }
        let single_step = self.ticks.is_some_and(|last| ticks == last + 1);
        self.ticks = Some(ticks);
        let tails = simulation
            .schedule()
            .map_or(0, |schedule| schedule.tails().len());
        self.slots.resize(tails, Slot::default());
        let mut seen = vec![false; tails];
        for view in simulation.flights() {
            let index = view.tail().as_usize();
            let Some(slot) = self.slots.get_mut(index) else {
                continue;
            };
            seen[index] = true;
            let pose = Pose {
                position: projection.project(view.position()),
                // Courses are in [0, 360); f32 is ample for drawing.
                #[allow(clippy::cast_possible_truncation)]
                heading: view.heading().value() as f32,
            };
            let new_leg = slot
                .view
                .is_none_or(|old| old.scheduled_departure() != view.scheduled_departure());
            if new_leg {
                slot.trail.clear();
            }
            // Interpolate across exactly one tick; after a jump, start afresh.
            slot.previous = if single_step && !new_leg {
                slot.current.or(Some(pose))
            } else {
                Some(pose)
            };
            slot.current = Some(pose);
            if view.phase().is_airborne() {
                slot.trail.push(simulation.time(), pose.position);
            }
            slot.view = Some(view);
        }
        for (slot, seen) in self.slots.iter_mut().zip(seen) {
            if !seen && slot.view.is_some() {
                *slot = Slot::default();
            }
        }
    }

    /// The pose of `tail` a fraction `t` of the way into the next tick.
    fn pose(&self, tail: Id<Tail>, t: f32) -> Option<Pose> {
        let slot = self.slots.get(tail.as_usize())?;
        Some(blend(slot.previous?, slot.current?, t))
    }
}

/// Systems that place the aircraft sprites; readers of their positions run
/// after.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DrawSet;

/// Draws the aircraft.
pub(crate) struct AircraftPlugin;

impl Plugin for AircraftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrafficView>()
            .add_systems(Startup, spawn_aircraft)
            .add_systems(
                Update,
                (
                    sample_traffic.after(SimSet),
                    draw_aircraft
                        .in_set(DrawSet)
                        .after(sample_traffic)
                        .after(CameraSet),
                    draw_trails.after(DrawSet),
                ),
            );
    }
}

fn spawn_aircraft(
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
    for class in SizeClass::ALL {
        match fill_outline(&outline(class)) {
            Ok(triangles) => shapes.push(meshes.add(triangles.into_mesh())),
            Err(error) => {
                error!("cannot draw aircraft: {error:#}");
                return;
            }
        }
    }
    let bands = BandMaterials(PhaseBand::ALL.map(|band| materials.add(band.color())));
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    for (tail, info) in schedule.tails() {
        let class = traffic
            .aircraft()
            .get(info.aircraft())
            .map_or(SizeClass::NarrowBody, size_class);
        commands.spawn((
            AircraftSprite::new(tail, Vec2::ZERO),
            SpriteLook {
                class,
                band: PhaseBand::Ground,
            },
            Mesh2d(shapes[class.index()].clone()),
            MeshMaterial2d(bands.0[PhaseBand::Ground.index()].clone()),
            Transform::from_xyz(0.0, 0.0, AIRCRAFT_Z),
            Visibility::Hidden,
            layer.clone(),
        ));
    }
    commands.insert_resource(bands);
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

fn draw_aircraft(
    rig: Option<Res<CameraRig>>,
    driver: Res<SimDriver>,
    projection: Res<MapProjection>,
    traffic: Res<TrafficView>,
    selection: Res<Selection>,
    bands: Option<Res<BandMaterials>>,
    mut sprites: Query<(
        &mut AircraftSprite,
        &mut SpriteLook,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial2d<ColorMaterial>,
    )>,
) {
    let (Some(rig), Some(bands)) = (rig, bands) else {
        return;
    };
    let scale = rig.state.scale();
    // Map units per metre barely change over Germany; the origin's is used.
    #[allow(clippy::cast_possible_truncation)]
    let units_per_meter = MapProjection::units_per_meter(projection.origin()) as f32;
    let t = tick_fraction(&driver);
    for (mut sprite, mut look, mut transform, mut visibility, mut material) in &mut sprites {
        let shown = traffic
            .flight(sprite.tail)
            .filter(|view| flight_visible(view.phase(), scale))
            .zip(traffic.pose(sprite.tail, t));
        let Some((view, pose)) = shown else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        sprite.position = pose.position;
        let highlighted = selection.selected_flight() == Some(sprite.tail)
            || selection.hovered_flight() == Some(sprite.tail);
        let z = if highlighted { HIGHLIGHT_Z } else { AIRCRAFT_Z };
        *transform = Transform::from_translation(pose.position.extend(z))
            .with_rotation(Quat::from_rotation_z(rotation(pose.heading)))
            .with_scale(Vec3::splat(display_length(
                pixel_length(look.class),
                real_length(look.class),
                scale,
                units_per_meter,
            )));
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
        let selected = selection.selected_flight() == Some(sprite.tail);
        let hovered = selection.hovered_flight() == Some(sprite.tail);
        let Some(slot) = traffic.slots.get(sprite.tail.as_usize()) else {
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
                .map(|(position, age)| (position, base.with_alpha(TRAIL_ALPHA * (1.0 - age))))
                .chain(std::iter::once((
                    sprite.position,
                    base.with_alpha(TRAIL_ALPHA),
                )));
            gizmos.linestrip_gradient_2d(points);
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
        let projection = MapProjection::germany()?;
        let mut traffic = TrafficView::default();
        traffic.sample(&driver, &projection);
        let airborne = traffic
            .slots
            .iter()
            .filter(|slot| slot.view.is_some_and(|view| view.phase().is_airborne()))
            .count();
        assert_eq!(airborne, driver.simulation().airborne());
        let (index, slot) = traffic
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.view.is_some_and(|view| view.phase().is_airborne()))
            .context("an airborne flight")?;
        let tail = Id::<Tail>::from_usize(index)?;
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
        let projection = MapProjection::germany()?;
        let mut traffic = TrafficView::default();
        traffic.sample(&driver, &projection);
        let before = traffic.slots.clone();
        traffic.sample(&driver, &projection);
        assert_eq!(traffic.slots, before);
        Ok(())
    }
}
