//! Arrows on the runways in use (plan M3 PR 5): towards the landing threshold
//! on arrival runways, away from the take-off threshold on departure ones.

use aerocosm_core::LatLon;
use bevy::{
    app::{App, Plugin, Update},
    ecs::{
        resource::Resource,
        schedule::{IntoScheduleConfigs, common_conditions::resource_added},
        system::{Commands, Res},
    },
    gizmos::gizmos::Gizmos,
    math::Vec2,
};

use crate::{
    camera::{CameraRig, CameraSet},
    driver::{SimDriver, SimReady},
    flights::PhaseBand,
    lod,
    projection::{MapProjection, wrap_near},
};

/// Arrow shaft length, metres.
pub(crate) const ARROW_LENGTH: f32 = 600.0;
/// Gap between the threshold and the arrow, metres.
const GAP: f32 = 150.0;
/// Arrow head: barb length relative to the shaft, and angle.
const HEAD: f32 = 0.35;
const BARB_DEGREES: f32 = 28.0;

/// What a runway end is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Use {
    /// Landing: the arrow points at the threshold from the approach side.
    Arrival,
    /// Take-off: the arrow points down the runway from the threshold.
    Departure,
}

/// Unit vector of a true course on the map (x east, y north).
fn direction(heading: f32) -> Vec2 {
    let radians = heading.to_radians();
    Vec2::new(radians.sin(), radians.cos())
}

/// Shaft and two barbs, as line segments in map units, for a runway end at
/// `threshold` used along `heading` (degrees).
pub(crate) fn arrow(
    threshold: Vec2,
    heading: f32,
    use_: Use,
    units_per_meter: f32,
) -> [(Vec2, Vec2); 3] {
    let along = direction(heading) * units_per_meter;
    let (start, end) = match use_ {
        Use::Arrival => (
            threshold - along * (GAP + ARROW_LENGTH),
            threshold - along * GAP,
        ),
        Use::Departure => (
            threshold + along * GAP,
            threshold + along * (GAP + ARROW_LENGTH),
        ),
    };
    let barb =
        |turn: f32| end + direction(heading + 180.0 + turn) * units_per_meter * ARROW_LENGTH * HEAD;
    [
        (start, end),
        (end, barb(BARB_DEGREES)),
        (end, barb(-BARB_DEGREES)),
    ]
}

/// Every active runway end: threshold, course, use.
#[derive(Resource, Debug, Default)]
struct ActiveArrows(Vec<(LatLon, f32, Use)>);

/// Arrows this many pixels from the screen centre are drawn: past the corner
/// of any window.
const VIEW_REACH: f32 = 3_000.0;

/// Draws arrows on the runways in use at runway zoom.
pub(crate) struct RunwayUsePlugin;

impl Plugin for RunwayUsePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                collect_arrows.run_if(resource_added::<SimDriver>),
                draw_arrows.after(CameraSet),
            )
                .in_set(SimReady),
        );
    }
}

/// The wind is fixed until weather arrives, so the runways in use are too.
fn collect_arrows(mut commands: Commands, driver: Res<SimDriver>) {
    let simulation = driver.simulation();
    let world = simulation.world();
    let mut arrows = Vec::new();
    for (id, _) in world.airports() {
        let runways = world.runways_of(id);
        for active in simulation.runways_in_use(id) {
            let Some(end) = runways
                .get(active.runway)
                .and_then(|r| r.ends().get(active.end))
            else {
                continue;
            };
            let position = end.threshold();
            // Courses are in [0, 360); f32 is ample for drawing.
            #[allow(clippy::cast_possible_truncation)]
            let heading = end.true_heading().value() as f32;
            if active.arrivals {
                arrows.push((position, heading, Use::Arrival));
            }
            if active.departures {
                arrows.push((position, heading, Use::Departure));
            }
        }
    }
    commands.insert_resource(ActiveArrows(arrows));
}

fn draw_arrows(
    rig: Option<Res<CameraRig>>,
    arrows: Option<Res<ActiveArrows>>,
    projection: Res<MapProjection>,
    mut gizmos: Gizmos,
) {
    let (Some(rig), Some(arrows)) = (rig, arrows) else {
        return;
    };
    if !lod::runways_visible(rig.state.scale()) {
        return;
    }
    let centre = rig.state.center();
    // Arrows within this distance of the camera, map units: the window and
    // an arrow's length.
    let reach = rig.state.scale() * VIEW_REACH;
    for (threshold, heading, use_) in &arrows.0 {
        let position = wrap_near(projection.project(*threshold), centre);
        if position.distance(centre) > reach {
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        let units_per_meter = MapProjection::units_per_meter(*threshold) as f32;
        let color = match use_ {
            Use::Arrival => PhaseBand::Descent.color(),
            Use::Departure => PhaseBand::Climb.color(),
        };
        for (from, to) in arrow(position, *heading, *use_, units_per_meter) {
            gizmos.line_2d(from, to, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;

    use super::*;

    #[test]
    fn test_arrival_arrow_lies_before_the_threshold_pointing_in() {
        // Landing northwards (heading 0) at the origin, 1 unit per metre.
        let lines = arrow(Vec2::ZERO, 0.0, Use::Arrival, 1.0);
        let shaft = lines[0];
        assert!(shaft.0.y < shaft.1.y && shaft.1.y <= 0.0, "{shaft:?}");
        assert!((shaft.1.y - shaft.0.y - ARROW_LENGTH).abs() < 1e-3);
        // The head sits at the shaft's tip.
        assert!(lines[1].0.distance(shaft.1) < 1e-3 && lines[2].0.distance(shaft.1) < 1e-3);
    }

    #[test]
    fn test_departure_arrow_runs_along_the_runway() {
        // Taking off eastwards (heading 90).
        let lines = arrow(Vec2::ZERO, 90.0, Use::Departure, 2.0);
        let shaft = lines[0];
        assert!(shaft.0.x >= 0.0 && shaft.1.x > shaft.0.x, "{shaft:?}");
        assert!((shaft.1.x - shaft.0.x - 2.0 * ARROW_LENGTH).abs() < 1e-3);
    }
}
