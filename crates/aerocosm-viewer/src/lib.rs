//! aerocosm viewer: a Bevy front end for the simulation, native and in the
//! browser. The binary runs [`run`]; [`bench`] serves the viewer's
//! benchmarks (plan M5 D139, D140).

// Bevy systems take `Res`/`Query` parameters by value and often need many of them, so these
// two pedantic lints fire on idiomatic code throughout this crate only (plan D11).
#![allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]

mod aircraft;
mod airports;
mod anchor;
pub mod bench;
mod camera;
mod data;
mod driver;
mod flights;
mod ground;
mod labels;
mod loading;
mod lod;
mod map;
mod measure;
mod picking;
mod pool;
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
    app::{App, Last, PluginGroup, Startup, Update},
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
    start::{Launch, StartView},
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

/// Opens the viewer's window with the view the command line asks for; with
/// `--measure`, plays the measured scenes and closes (plan M9 D182).
pub fn run() -> Result<()> {
    let launch = Launch::from_args(std::env::args().skip(1))?;
    let view = launch.view;
    let mut app = App::new();
    app.add_plugins(
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
    );
    assemble(&mut app, view)?;
    if let Some(measure) = launch.measure {
        app.add_plugins(measure::MeasurePlugin(measure));
    }
    app.run();
    Ok(())
}

/// Adds the viewer's data, resources and plugins to `app`, which already has
/// Bevy's plugins: shared by the window and the headless benchmark.
fn assemble(app: &mut App, view: StartView) -> Result<()> {
    let (scenario, map) = data::load()?;
    let start: SimTime = START.parse().context("invalid start time")?;
    // The viewer and the simulation each keep a copy of the static world; it is
    // small and never mutated, and this keeps the core free of shared-ownership
    // types.
    let world = scenario.world().clone();
    app.insert_resource(MapProjection::new(view.centre))
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
        .add_systems(Last, dispatch_ticks);
    Ok(())
}

/// Takes the simulation back from its background ticks, once they have
/// finished (plan M9 PR 5): the frame's readers then see the new tick.
fn drive_simulation(mut driver: ResMut<SimDriver>, mut last: ResMut<driver::Advance>) {
    *last = driver::Advance::default();
    match driver.collect() {
        Some(Ok(frame)) => *last = frame,
        Some(Err(error)) => {
            error!("{error:#}");
            driver.set_speed(aerocosm_core::SimSpeed::Paused);
        }
        None => {}
    }
}

/// Starts the ticks due by now in the background, once every system of the
/// frame has read the simulation (plan M9 PR 5): they run while the frame
/// is drawn and the next one begins.
fn dispatch_ticks(time: Res<Time<Real>>, driver: Option<ResMut<SimDriver>>) {
    if let Some(mut driver) = driver {
        driver.dispatch(time.elapsed());
    }
}
