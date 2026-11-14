//! The ground layouts, loaded at start (plan M5 D132): every region file is
//! read through Bevy's asset server, from `assets/ground` on disk natively
//! and over HTTP on the web, and the simulation is built once all are in, so
//! ground control behaves as in the runner wherever the camera is.

use aerocosm_core::{
    RngSeed, SimConfig, SimTime, Simulation,
    ground::{GroundDefinition, ground_file},
    scenario::Scenario,
};
use anyhow::{Context, Result};
use bevy::{
    app::{App, Plugin, Startup, Update},
    asset::{
        Asset, AssetApp, AssetLoader, AssetServer, Assets, Handle, LoadContext,
        RecursiveDependencyLoadState, io::Reader,
    },
    color::Alpha,
    ecs::{
        children,
        component::Component,
        entity::Entity,
        hierarchy::Children,
        query::With,
        resource::Resource,
        schedule::{
            IntoScheduleConfigs,
            common_conditions::{not, resource_exists},
        },
        system::{Commands, Query, Res, ResMut},
    },
    log::{error, info},
    math::Vec2,
    reflect::TypePath,
    text::{TextColor, TextFont},
    ui::{
        JustifyContent, Node, PositionType, percent,
        widget::{Text, TextShadow},
    },
    utils::default,
};

use crate::{data::Fonts, driver::SimDriver, theme};

/// The ICAO regions with ground layouts: the files in `assets/ground` (a
/// guard test keeps the two in step).
pub(crate) const REGIONS: [char; 21] = [
    'B', 'C', 'D', 'E', 'F', 'G', 'H', 'K', 'L', 'M', 'N', 'O', 'P', 'R', 'S', 'T', 'U', 'V', 'W',
    'Y', 'Z',
];

/// One region's ground layouts.
#[derive(Asset, TypePath, Debug)]
pub(crate) struct GroundRegion(GroundDefinition);

/// Decodes `*.ground.bin` files.
#[derive(Default, TypePath)]
struct GroundRegionLoader;

impl AssetLoader for GroundRegionLoader {
    type Asset = GroundRegion;
    type Error = String;
    type Settings = ();

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<GroundRegion, String> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| format!("{}: {error}", context.path()))?;
        GroundDefinition::from_asset(&bytes)
            .map(GroundRegion)
            .map_err(|error| format!("{}: {error:#}", context.path()))
    }

    fn extensions(&self) -> &[&str] {
        &["ground.bin"]
    }
}

/// How far loading has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Progress {
    /// Files loaded or failed.
    pub(crate) done: usize,
    pub(crate) total: usize,
}

impl Progress {
    /// Progress over files in `states`: `Some(true)` loaded, `Some(false)`
    /// failed, `None` still loading.
    pub(crate) fn of(states: impl IntoIterator<Item = Option<bool>>) -> Self {
        let (mut done, mut total) = (0, 0);
        for state in states {
            total += 1;
            done += usize::from(state.is_some());
        }
        Self { done, total }
    }

    /// Whether every file has loaded or failed.
    pub(crate) const fn finished(self) -> bool {
        self.done == self.total
    }
}

/// The simulation's inputs while the ground loads.
#[derive(Resource)]
pub(crate) struct PendingSimulation {
    scenario: Option<Scenario>,
    config: SimConfig,
    regions: Vec<Handle<GroundRegion>>,
}

impl PendingSimulation {
    /// A simulation of `scenario` (without ground) from `start`.
    pub(crate) const fn new(scenario: Scenario, start: SimTime) -> Self {
        Self {
            scenario: Some(scenario),
            config: SimConfig::new(start, RngSeed::new(0)),
            regions: Vec::new(),
        }
    }
}

/// `scenario` with the layouts of `regions`.
pub(crate) fn with_regions(
    scenario: Scenario,
    regions: impl IntoIterator<Item = GroundDefinition>,
) -> Result<Scenario> {
    scenario.with_ground(GroundDefinition::combine(regions).context("ground regions overlap")?)
}

/// The loading notice.
#[derive(Component)]
struct LoadingNotice;

/// Loads the ground and builds the simulation.
pub(crate) struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<GroundRegion>()
            .register_asset_loader(GroundRegionLoader)
            .add_systems(
                Startup,
                (
                    request_regions,
                    spawn_notice.after(crate::data::setup_fonts),
                ),
            )
            .add_systems(
                Update,
                finish_loading.run_if(not(resource_exists::<SimDriver>)),
            );
    }
}

fn request_regions(server: Res<AssetServer>, mut pending: ResMut<PendingSimulation>) {
    pending.regions = REGIONS
        .iter()
        .map(|region| server.load(format!("ground/{}", ground_file(Some(*region)))))
        .collect();
}

fn spawn_notice(mut commands: Commands, fonts: Res<Fonts>) {
    commands.spawn((
        LoadingNotice,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: percent(46),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            Text::new("Loading airport layouts…"),
            TextFont::from_font_size(15.0).with_font(fonts.semibold.clone()),
            TextColor(theme::TEXT),
            TextShadow {
                offset: Vec2::new(0.0, 1.0),
                color: theme::SEA.with_alpha(0.9),
            },
        )],
    ));
}

fn finish_loading(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut regions: ResMut<Assets<GroundRegion>>,
    pending: Option<ResMut<PendingSimulation>>,
    notice: Query<Entity, With<LoadingNotice>>,
    mut texts: Query<&mut Text>,
    children: Query<&Children>,
) {
    let Some(mut pending) = pending else {
        return;
    };
    let progress = Progress::of(pending.regions.iter().map(|handle| {
        match server.get_recursive_dependency_load_state(handle) {
            Some(RecursiveDependencyLoadState::Loaded) => Some(true),
            Some(RecursiveDependencyLoadState::Failed(_)) | None => Some(false),
            Some(_) => None,
        }
    }));
    for entity in &notice {
        for child in children.iter_descendants(entity) {
            if let Ok(mut text) = texts.get_mut(child) {
                let line = format!(
                    "Loading airport layouts… {} of {}",
                    progress.done, progress.total
                );
                if text.0 != line {
                    text.0 = line;
                }
            }
        }
    }
    if !progress.finished() {
        return;
    }
    let definitions: Vec<GroundDefinition> = pending
        .regions
        .drain(..)
        .filter_map(|handle| {
            let region = regions.remove(&handle);
            if region.is_none() {
                error!("{handle:?}: no ground layouts, its airports taxi without them");
            }
            region.map(|GroundRegion(definition)| definition)
        })
        .collect();
    let Some(scenario) = pending.scenario.take() else {
        return;
    };
    let config = pending.config;
    info!(regions = definitions.len(), "ground layouts loaded");
    let built = with_regions(scenario, definitions)
        .and_then(|scenario| Simulation::new(config, scenario))
        .and_then(SimDriver::new);
    match built {
        Ok(driver) => {
            info!(
                airports = driver.simulation().scenario().ground().airports().len(),
                "simulation ready"
            );
            commands.insert_resource(driver);
            commands.remove_resource::<PendingSimulation>();
            for entity in &notice {
                commands.entity(entity).despawn();
            }
        }
        Err(error) => error!("cannot start the simulation: {error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::{Context, Result};

    use super::*;
    use crate::data;

    #[test]
    fn test_progress_counts_loaded_and_failed_files() {
        let progress = Progress::of([Some(true), None, Some(false)]);
        assert_eq!(progress, Progress { done: 2, total: 3 });
        assert!(!progress.finished());
        assert!(Progress::of([Some(true), Some(false)]).finished());
        assert!(Progress::of([]).finished());
    }

    #[test]
    fn test_regions_are_the_shipped_ground_files() -> Result<()> {
        let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/ground");
        let mut shipped: Vec<String> = fs::read_dir(directory)
            .context("assets/ground")?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".ground.bin"))
            .collect();
        shipped.sort();
        let listed: Vec<String> = REGIONS.iter().map(|r| ground_file(Some(*r))).collect();
        assert_eq!(listed, shipped);
        Ok(())
    }

    #[test]
    fn test_with_regions_adds_the_layouts() -> Result<()> {
        let (scenario, _) = data::load_with_ground(&['Z'])?;
        let pudong = scenario
            .world()
            .airport_by_ident("ZSPD".parse()?)
            .context("ZSPD")?;
        assert!(scenario.ground().airport(pudong).is_some());
        Ok(())
    }
}
