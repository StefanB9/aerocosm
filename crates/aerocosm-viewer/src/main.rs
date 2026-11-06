//! aerocosm viewer: a Bevy front end for the simulation, native and in the
//! browser.

// Bevy systems take `Res`/`Query` parameters by value and often need many of them, so these
// two pedantic lints fire on idiomatic code throughout this crate only (plan D11).
#![allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]

mod airports;
mod camera;
mod data;
mod driver;
mod labels;
mod lod;
mod map;
mod picking;
mod projection;
mod tessellate;
mod theme;
mod ui;

use aerocosm_core::{RngSeed, Scenario, SimConfig, SimTime, Simulation};
use anyhow::{Context, Result};
use bevy::{
    DefaultPlugins,
    app::{App, PluginGroup, Startup, Update},
    ecs::system::{Res, ResMut},
    log::error,
    time::{Real, Time},
    utils::default,
    window::{Window, WindowPlugin, WindowResolution},
};

use crate::{
    airports::{AirportsPlugin, WorldView},
    camera::CameraPlugin,
    driver::SimDriver,
    map::MapPlugin,
    projection::MapProjection,
    ui::UiPlugin,
};

/// Simulated start time. The browser has no portable wall-clock date in `std`,
/// so the viewer starts at a fixed morning; later milestones make this
/// configurable.
const START: &str = "2026-11-05T06:00:00Z";

fn main() -> Result<()> {
    let (world, map) = data::load()?;
    let start: SimTime = START.parse().context("invalid start time")?;
    // The viewer and the simulation each keep a copy of the static world; it is
    // small and never mutated, and this keeps the core free of shared-ownership
    // types.
    let simulation = Simulation::new(
        SimConfig::new(start, RngSeed::new(0)),
        Scenario::new(world.clone()),
    )?;
    let driver = SimDriver::new(simulation)?;

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "aerocosm".to_owned(),
                resolution: WindowResolution::new(1440, 900),
                canvas: Some("#aerocosm".to_owned()),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(MapProjection::germany()?)
        .insert_resource(map)
        .insert_resource(WorldView { world })
        .insert_resource(driver)
        .add_systems(Startup, data::setup_fonts)
        .add_plugins((CameraPlugin, MapPlugin, AirportsPlugin, UiPlugin))
        .add_systems(Update, drive_simulation)
        .run();
    Ok(())
}

/// Advances the simulation by the real time elapsed, at the selected speed.
fn drive_simulation(time: Res<Time<Real>>, mut driver: ResMut<SimDriver>) {
    if let Err(error) = driver.advance(time.elapsed()) {
        error!("{error:#}");
        driver.set_speed(aerocosm_core::SimSpeed::Paused);
    }
}
