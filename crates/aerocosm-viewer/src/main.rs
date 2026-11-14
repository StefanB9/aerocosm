//! aerocosm viewer: a Bevy front end for the simulation, native and in the
//! browser.

// Bevy systems take `Res`/`Query` parameters by value and often need many of them, so these
// two pedantic lints fire on idiomatic code throughout this crate only (plan D11).
#![allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]

mod aircraft;
mod airports;
mod anchor;
mod camera;
mod data;
mod driver;
mod flights;
mod ground;
mod labels;
mod loading;
mod lod;
mod map;
mod picking;
mod projection;
mod runway_use;
mod selection;
mod silhouette;
mod start;
mod tessellate;
mod theme;
mod ui;

use aerocosm_core::SimTime;
use anyhow::{Context, Result};
use bevy::{
    DefaultPlugins,
    app::{App, PluginGroup, Startup, Update},
    asset::{AssetMetaCheck, AssetPlugin},
    ecs::{
        schedule::{IntoScheduleConfigs, common_conditions::resource_exists},
        system::{Res, ResMut},
    },
    log::error,
    time::{Real, Time},
    utils::default,
    window::{Window, WindowPlugin, WindowResolution},
};

use crate::{
    aircraft::AircraftPlugin,
    airports::{AirportsPlugin, WorldView},
    anchor::AnchorPlugin,
    camera::CameraPlugin,
    driver::{SimDriver, SimReady, SimSet},
    ground::GroundPlugin,
    loading::{LoadingPlugin, PendingSimulation},
    map::MapPlugin,
    projection::MapProjection,
    runway_use::RunwayUsePlugin,
    start::StartView,
    ui::UiPlugin,
};

/// Simulated start time (D52): a fixed instant in the morning wave, 08:00 in
/// Germany, identical on desktop and web.
const START: &str = "2026-11-05T07:00:00Z";

/// The repository's `assets` folder: natively relative to the crate (under
/// `cargo run`) or to `target/<profile>`; on the web next to the page, where
/// `cargo xtask web build` copies the ground layouts.
#[cfg(not(target_arch = "wasm32"))]
const ASSETS: &str = "../../assets";
#[cfg(target_arch = "wasm32")]
const ASSETS: &str = "assets";

fn main() -> Result<()> {
    let view = StartView::from_args(std::env::args().skip(1))?;
    let (scenario, map) = data::load()?;
    let start: SimTime = START.parse().context("invalid start time")?;
    // The viewer and the simulation each keep a copy of the static world; it is
    // small and never mutated, and this keeps the core free of shared-ownership
    // types.
    let world = scenario.world().clone();

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "aerocosm".to_owned(),
                        resolution: WindowResolution::new(1440, 900),
                        canvas: Some("#aerocosm".to_owned()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: ASSETS.to_owned(),
                    // No `.meta` files: on the web each would be a request.
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .insert_resource(MapProjection::new(view.centre))
        .insert_resource(view)
        .insert_resource(map)
        .insert_resource(WorldView { world })
        .insert_resource(PendingSimulation::new(scenario, start))
        .add_systems(Startup, data::setup_fonts)
        .configure_sets(Update, SimReady.run_if(resource_exists::<SimDriver>))
        .add_plugins((
            LoadingPlugin,
            CameraPlugin,
            AnchorPlugin,
            MapPlugin,
            AirportsPlugin,
            GroundPlugin,
            AircraftPlugin,
            RunwayUsePlugin,
            UiPlugin,
        ))
        .add_systems(Update, drive_simulation.in_set(SimSet).in_set(SimReady))
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
