//! Data compiled into the binary (plan D26): identical on desktop and web, no
//! loading state.

use aerocosm_basemap::Basemap;
use aerocosm_core::{
    AircraftCatalog, Scenario,
    airline::AirlineCatalog,
    ground::{GroundDefinition, StandAreasDefinition},
    traffic::TrafficTargets,
    world::World,
};
use anyhow::{Context, Result};
use bevy::{
    asset::{Assets, Handle},
    ecs::{
        resource::Resource,
        system::{Commands, ResMut},
    },
    text::Font,
};

const WORLD_JSON: &str = include_str!("../../../assets/world/germany.world.json");
const AIRCRAFT_JSON: &str = include_str!("../../../assets/aircraft/types.json");
const AIRLINES_JSON: &str = include_str!("../../../assets/airlines/airlines.json");
const TRAFFIC_JSON: &str = include_str!("../../../assets/traffic/germany.json");
const GROUND_JSON: &str = include_str!("../../../assets/ground/germany.ground.json");
const STAND_AREAS_JSON: &str = include_str!("../../../assets/ground/stand-areas.json");
const BASEMAP_JSON: &str = include_str!("../../../assets/basemap/germany.basemap.json");
const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// Render layer of the basemap (land, water, lines).
pub(crate) const MAP_LAYER: usize = 0;
/// Render layer drawn above the basemap (runways, airports).
pub(crate) const OVERLAY_LAYER: usize = 1;

/// The static map data.
#[derive(Resource)]
pub(crate) struct MapData {
    pub(crate) basemap: Basemap,
}

/// Parses the embedded world, traffic inputs and basemap.
pub(crate) fn load() -> Result<(Scenario, MapData)> {
    let world = World::from_json(WORLD_JSON).context("embedded world file is invalid")?;
    let scenario = Scenario::with_traffic(
        world,
        AircraftCatalog::from_json(AIRCRAFT_JSON).context("embedded aircraft are invalid")?,
        AirlineCatalog::from_json(AIRLINES_JSON).context("embedded airlines are invalid")?,
        TrafficTargets::from_json(TRAFFIC_JSON).context("embedded traffic is invalid")?,
    )?
    .with_ground(GroundDefinition::from_json(GROUND_JSON).context("embedded ground is invalid")?)?
    .with_stand_areas(
        StandAreasDefinition::from_json(STAND_AREAS_JSON)
            .context("embedded stand areas are invalid")?,
    )?;
    let basemap = Basemap::from_json(BASEMAP_JSON).context("embedded basemap is invalid")?;
    Ok((scenario, MapData { basemap }))
}

/// The UI typefaces.
#[derive(Resource, Clone)]
pub(crate) struct Fonts {
    pub(crate) regular: Handle<Font>,
    pub(crate) semibold: Handle<Font>,
}

/// Registers the embedded Inter faces.
pub(crate) fn setup_fonts(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    commands.insert_resource(Fonts {
        regular: fonts.add(Font::from_bytes(INTER_REGULAR.to_vec())),
        semibold: fonts.add(Font::from_bytes(INTER_SEMIBOLD.to_vec())),
    });
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_embedded_data_loads() -> Result<()> {
        let (scenario, map) = load()?;
        assert!(scenario.world().airport_count() > 400);
        let traffic = scenario.traffic().context("embedded traffic")?;
        assert!(traffic.airlines().len() > 20);
        assert!(scenario.ground().airports().len() > 20);
        assert!(map.basemap.countries().len() > 10);
        Ok(())
    }
}
