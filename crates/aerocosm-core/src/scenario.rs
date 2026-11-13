//! What a run simulates (D45): the world and, optionally, the traffic inputs
//! a schedule is generated from.

use anyhow::{Context, Result};

use crate::{
    AircraftCatalog,
    airline::AirlineCatalog,
    ground::{GroundDefinition, GroundNetwork, StandAreas, StandAreasDefinition, TaxiTimes},
    schedule::ScheduleInputs,
    traffic::TrafficTargets,
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
    ground: GroundNetwork,
    areas: StandAreas,
    /// Typical taxi times by airport index; empty without ground layouts.
    taxi: Vec<Option<TaxiTimes>>,
}

impl Scenario {
    /// A static world without traffic.
    pub const fn new(world: World) -> Self {
        Self {
            world,
            traffic: None,
            ground: GroundNetwork::empty(),
            areas: StandAreas::empty(),
            taxi: Vec::new(),
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
            ground: GroundNetwork::empty(),
            areas: StandAreas::empty(),
            taxi: Vec::new(),
        })
    }

    /// The scenario with airport ground layouts (plan M4), validated against
    /// its world. Airports without a layout keep straight-line ground.
    pub fn with_ground(mut self, definition: GroundDefinition) -> Result<Self> {
        self.ground = GroundNetwork::from_definition(definition, &self.world)
            .context("ground layouts do not match the world")?;
        let targets = self.traffic.as_ref().map(|traffic| &traffic.targets);
        self.taxi = self.ground.taxi_times(&self.world, targets);
        Ok(self)
    }

    /// The scenario with curated stand areas (plan M4 D68).
    pub fn with_stand_areas(mut self, definition: StandAreasDefinition) -> Result<Self> {
        self.areas = StandAreas::new(definition, &self.world)
            .context("stand areas do not match the world")?;
        Ok(self)
    }

    /// The curated stand areas.
    pub const fn stand_areas(&self) -> &StandAreas {
        &self.areas
    }

    /// Typical taxi times by airport index (plan M4 D73); empty without
    /// ground layouts.
    pub fn taxi_times(&self) -> &[Option<TaxiTimes>] {
        &self.taxi
    }

    /// The airport ground layouts.
    pub const fn ground(&self) -> &GroundNetwork {
        &self.ground
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
        .map(|inputs| inputs.with_taxi(&self.taxi))
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
        assert_eq!(traffic.aircraft().len(), 37);
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

    #[test]
    fn test_scenario_carries_a_ground_network() -> Result<()> {
        let data = data()?;
        let scenario = Scenario::new(data.world);
        assert_eq!(scenario.ground().airports().len(), 0);
        let definition = crate::ground::GroundDefinition {
            format_version: crate::ground::FORMAT_VERSION,
            attribution: "© OpenStreetMap contributors".to_owned(),
            airports: vec![crate::ground::definition_sample()?],
        };
        let scenario = scenario.with_ground(definition)?;
        assert_eq!(scenario.ground().airports().len(), 1);
        assert_eq!(
            scenario.ground().attribution(),
            "© OpenStreetMap contributors"
        );
        // Layouts are checked against the scenario's own world.
        let mismatched = crate::ground::GroundDefinition {
            format_version: crate::ground::FORMAT_VERSION,
            attribution: String::new(),
            airports: vec![crate::ground::definition_sample()?],
        };
        assert!(Scenario::empty().with_ground(mismatched).is_err());
        Ok(())
    }

    #[test]
    fn test_scenario_carries_stand_areas() -> Result<()> {
        let data = data()?;
        let frankfurt = data
            .world
            .airport_by_ident("EDDF".parse()?)
            .ok_or_else(|| anyhow::anyhow!("EDDF"))?;
        let scenario = Scenario::new(data.world);
        assert_eq!(scenario.stand_areas().areas(frankfurt).len(), 0);
        let definition = crate::ground::StandAreasDefinition::from_json(include_str!(
            "../../../assets/ground/stand-areas.json"
        ))?;
        let scenario = scenario.with_stand_areas(definition.clone())?;
        assert_ne!(scenario.stand_areas().areas(frankfurt).len(), 0);
        assert!(Scenario::empty().with_stand_areas(definition).is_err());
        Ok(())
    }

    #[test]
    fn test_scenario_with_ground_feeds_taxi_times_to_schedules() -> Result<()> {
        let data = data()?;
        let frankfurt = data
            .world
            .airport_by_ident("EDDF".parse()?)
            .ok_or_else(|| anyhow::anyhow!("EDDF"))?;
        let plain = Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        assert_eq!(plain.taxi_times().len(), 0);
        let mapped = plain.with_ground(crate::ground::GroundDefinition::from_json(
            include_str!("../../../assets/ground/germany.ground.json"),
        )?)?;
        let times = mapped.taxi_times();
        assert!(times[frankfurt.as_usize()].is_some());
        assert!(mapped.schedule_inputs().is_some());
        Ok(())
    }
}
