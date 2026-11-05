//! Data compiled into the binary (plan D26): identical on desktop and web, no
//! loading state.

use aerocosm_basemap::Basemap;
use aerocosm_core::world::World;
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

/// Parses the embedded world and basemap.
pub(crate) fn load() -> Result<(World, MapData)> {
    let world = World::from_json(WORLD_JSON).context("embedded world file is invalid")?;
    let basemap = Basemap::from_json(BASEMAP_JSON).context("embedded basemap is invalid")?;
    Ok((world, MapData { basemap }))
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
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_embedded_data_loads() -> Result<()> {
        let (world, map) = load()?;
        assert!(world.airport_count() > 400);
        assert!(map.basemap.countries().len() > 10);
        Ok(())
    }
}
