//! The shipped data files, loaded for tests across modules.

use anyhow::Result;

use crate::{AircraftCatalog, airline::AirlineCatalog, traffic::TrafficTargets, world::World};

/// World, catalogs and traffic targets as committed in `assets/`.
pub(crate) struct Data {
    pub(crate) world: World,
    pub(crate) aircraft: AircraftCatalog,
    pub(crate) airlines: AirlineCatalog,
    pub(crate) traffic: TrafficTargets,
}

/// Loads the shipped data.
pub(crate) fn data() -> Result<Data> {
    Ok(Data {
        world: World::from_json(include_str!("../../../assets/world/germany.world.json"))?,
        aircraft: AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?,
        airlines: AirlineCatalog::from_json(include_str!(
            "../../../assets/airlines/airlines.json"
        ))?,
        traffic: TrafficTargets::from_json(include_str!("../../../assets/traffic/germany.json"))?,
    })
}
