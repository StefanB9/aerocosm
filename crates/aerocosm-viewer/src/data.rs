//! Data compiled into the binary (plan D26): identical on desktop and web, no
//! loading state.

use aerocosm_basemap::Basemap;
use aerocosm_core::{
    AircraftCatalog, Scenario,
    ground::GroundDefinition,
    traffic::{model::derive, observed::ObservedTraffic},
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

const WORLD: &[u8] = include_bytes!("../../../assets/world/world.bin");
const AIRCRAFT_JSON: &str = include_str!("../../../assets/aircraft/types.json");
const OBSERVED: &[u8] = include_bytes!("../../../assets/traffic/observed.bin");
/// The ground of northern and southern Europe (ICAO regions E and L) until
/// PR 4 streams it per airport (plan M5 D124).
const GROUND: [&[u8]; 2] = [
    include_bytes!("../../../assets/ground/E.ground.bin"),
    include_bytes!("../../../assets/ground/L.ground.bin"),
];
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

/// Parses the embedded world and basemap and derives the world model
/// (plan M5).
pub(crate) fn load() -> Result<(Scenario, MapData)> {
    let world = World::from_asset(WORLD).context("embedded world is invalid")?;
    let aircraft =
        AircraftCatalog::from_json(AIRCRAFT_JSON).context("embedded aircraft are invalid")?;
    let observed =
        ObservedTraffic::from_asset(OBSERVED).context("embedded observed traffic is invalid")?;
    let (targets, airlines) = derive(&world, &observed, &aircraft)
        .context("cannot derive the world model")?
        .into_parts();
    let scenario = Scenario::with_traffic(world, aircraft, airlines, targets)?.with_ground(
        GroundDefinition::combine(
            GROUND
                .iter()
                .map(|bytes| GroundDefinition::from_asset(bytes))
                .collect::<Result<Vec<_>>>()
                .context("embedded ground is invalid")?,
        )
        .context("embedded ground is invalid")?,
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
        assert!(scenario.world().airport_count() > 20_000);
        let traffic = scenario.traffic().context("embedded traffic")?;
        assert!(traffic.airlines().len() > 300);
        assert!(scenario.ground().airports().len() > 20);
        assert!(map.basemap.countries().len() > 10);
        Ok(())
    }
}
