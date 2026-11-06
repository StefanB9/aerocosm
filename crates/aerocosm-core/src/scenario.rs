//! What a run simulates (D45): the world and, optionally, the traffic inputs
//! a schedule is generated from.

use anyhow::{Context, Result};

use crate::{
    AircraftCatalog, airline::AirlineCatalog, schedule::ScheduleInputs, traffic::TrafficTargets,
    world::World,
};

/// Aircraft, airlines and calibration targets, checked against the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Traffic {
    aircraft: AircraftCatalog,
    airlines: AirlineCatalog,
    targets: TrafficTargets,
}

impl Traffic {
    /// The aircraft catalog.
    pub const fn aircraft(&self) -> &AircraftCatalog {
        &self.aircraft
    }

    /// The airline catalog.
    pub const fn airlines(&self) -> &AirlineCatalog {
        &self.airlines
    }

    /// Movement targets, zones, curfews and the airspace.
    pub const fn targets(&self) -> &TrafficTargets {
        &self.targets
    }
}

/// The inputs of a run besides its config.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    world: World,
    traffic: Option<Traffic>,
}

impl Scenario {
    /// A static world without traffic.
    pub const fn new(world: World) -> Self {
        Self {
            world,
            traffic: None,
        }
    }

    /// An empty world without traffic.
    pub fn empty() -> Self {
        Self::new(World::empty())
    }

    /// A world with traffic, after checking that every reference between the
    /// inputs resolves.
    pub fn with_traffic(
        world: World,
        aircraft: AircraftCatalog,
        airlines: AirlineCatalog,
        targets: TrafficTargets,
    ) -> Result<Self> {
        ScheduleInputs::new(&world, &aircraft, &airlines, &targets)
            .context("traffic inputs do not match the world")?;
        Ok(Self {
            world,
            traffic: Some(Traffic {
                aircraft,
                airlines,
                targets,
            }),
        })
    }

    /// The static world.
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// The traffic inputs, if any.
    pub const fn traffic(&self) -> Option<&Traffic> {
        self.traffic.as_ref()
    }

    /// Inputs for schedule generation, if the scenario has traffic.
    pub(crate) fn schedule_inputs(&self) -> Option<ScheduleInputs<'_>> {
        let traffic = self.traffic.as_ref()?;
        // SAFETY: the references were checked in `with_traffic`, and the
        // fields are immutable since.
        ScheduleInputs::new(
            &self.world,
            &traffic.aircraft,
            &traffic.airlines,
            &traffic.targets,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::test_data::data;

    #[test]
    fn test_scenario_world_only_has_no_traffic() {
        let scenario = Scenario::new(World::empty());
        assert_eq!(scenario.world().airport_count(), 0);
        assert!(scenario.traffic().is_none());
        assert!(scenario.schedule_inputs().is_none());
        assert_eq!(Scenario::empty(), scenario);
    }

    #[test]
    fn test_scenario_with_traffic_exposes_inputs() -> Result<()> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        let traffic = scenario
            .traffic()
            .ok_or_else(|| anyhow::anyhow!("traffic"))?;
        assert_eq!(traffic.aircraft().len(), 23);
        assert_eq!(traffic.airlines().len(), 23);
        assert!(traffic.targets().airports().len() > 20);
        assert!(scenario.schedule_inputs().is_some());
        Ok(())
    }

    #[test]
    fn test_scenario_rejects_mismatched_inputs() -> Result<()> {
        let data = data()?;
        let result =
            Scenario::with_traffic(World::empty(), data.aircraft, data.airlines, data.traffic);
        assert!(result.is_err());
        Ok(())
    }
}
