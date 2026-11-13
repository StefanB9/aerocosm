//! The shipped data, loaded for tests across modules: the world and the
//! traffic model derived from it (plan M5).

use anyhow::Result;

use crate::{
    AircraftCatalog,
    airline::AirlineCatalog,
    traffic::{TrafficTargets, model::derive, observed::ObservedTraffic},
    world::World,
};

/// World, catalogs and traffic targets as derived from `assets/`.
pub(crate) struct Data {
    pub(crate) world: World,
    pub(crate) aircraft: AircraftCatalog,
    pub(crate) airlines: AirlineCatalog,
    pub(crate) traffic: TrafficTargets,
}

/// Loads the shipped data and derives the world model.
pub(crate) fn data() -> Result<Data> {
    let world = World::from_asset(include_bytes!("../../../assets/world/world.bin"))?;
    let aircraft = AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?;
    let observed =
        ObservedTraffic::from_asset(include_bytes!("../../../assets/traffic/observed.bin"))?;
    let (traffic, airlines) = derive(&world, &observed, &aircraft)?.into_parts();
    Ok(Data {
        world,
        aircraft,
        airlines,
        traffic,
    })
}
