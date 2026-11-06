//! Airport ground layouts from `OpenStreetMap` (plan M4): taxi networks,
//! stands, holding positions and runway exits.

use anyhow::{Context, Result};

use crate::{
    CoreError, Id,
    world::{Airport, World},
};

mod airport;
mod definition;

pub use airport::{AirportGround, Edge, Exit, HoldingPosition, Node, Stand};
pub use definition::{
    AirportGroundDefinition, EdgeDefinition, EdgeKind, ExitDefinition, FORMAT_VERSION,
    GroundDefinition, HoldingPositionDefinition, Point, StandDefinition,
};

pub use crate::world::codes::GroundName;

/// A one-airport layout at Frankfurt, for tests elsewhere in the crate.
#[cfg(test)]
pub(crate) fn definition_sample() -> Result<AirportGroundDefinition> {
    definition::tests::sample()
}

/// The ground layouts of all mapped airports, indexed by airport.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroundNetwork {
    attribution: String,
    grounds: Vec<AirportGround>,
    /// Position in `grounds` per world airport index.
    by_airport: Vec<Option<u32>>,
}

impl GroundNetwork {
    /// No mapped airports.
    pub const fn empty() -> Self {
        Self {
            attribution: String::new(),
            grounds: Vec::new(),
            by_airport: Vec::new(),
        }
    }

    /// Validates every airport's layout against `world`.
    pub fn from_definition(definition: GroundDefinition, world: &World) -> Result<Self> {
        let mut grounds = Vec::with_capacity(definition.airports.len());
        let mut by_airport = vec![None; world.airport_count()];
        for airport in definition.airports {
            let ident = airport.airport.to_string();
            let ground = AirportGround::new(airport, world)
                .with_context(|| format!("invalid ground layout for {ident}"))?;
            let slot = by_airport
                .get_mut(ground.airport().as_usize())
                .context("airport id out of range")?;
            if slot.is_some() {
                return Err(CoreError::InvalidGround {
                    airport: ident,
                    reason: "the airport is listed twice",
                }
                .into());
            }
            *slot = Some(u32::try_from(grounds.len()).context("too many ground layouts")?);
            grounds.push(ground);
        }
        Ok(Self {
            attribution: definition.attribution,
            grounds,
            by_airport,
        })
    }

    /// Parses and validates a ground file against `world`.
    pub fn from_json(text: &str, world: &World) -> Result<Self> {
        Self::from_definition(GroundDefinition::from_json(text)?, world)
    }

    /// Attribution the data licence requires wherever the layouts are shown.
    pub fn attribution(&self) -> &str {
        &self.attribution
    }

    /// The layout of `airport`, if it is mapped.
    pub fn airport(&self, airport: Id<Airport>) -> Option<&AirportGround> {
        let index = (*self.by_airport.get(airport.as_usize())?)?;
        self.grounds.get(index as usize)
    }

    /// All mapped airports' layouts.
    pub fn airports(&self) -> &[AirportGround] {
        &self.grounds
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::{definition::tests::sample, *};
    use crate::test_data::data;

    fn definition() -> Result<GroundDefinition> {
        Ok(GroundDefinition {
            format_version: FORMAT_VERSION,
            attribution: "© OpenStreetMap contributors".to_owned(),
            airports: vec![sample()?],
        })
    }

    #[test]
    fn test_ground_network_indexes_airports() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(&definition()?.to_json()?, &world)?;
        assert_eq!(network.attribution(), "© OpenStreetMap contributors");
        assert_eq!(network.airports().len(), 1);
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let munich = world.airport_by_ident("EDDM".parse()?).context("EDDM")?;
        assert_eq!(
            network.airport(frankfurt).map(AirportGround::airport),
            Some(frankfurt)
        );
        assert!(network.airport(munich).is_none());
        assert!(network.airport(Id::new(u32::MAX)).is_none());
        assert_eq!(GroundNetwork::empty().airports().len(), 0);
        Ok(())
    }

    #[test]
    fn test_ground_network_rejects_duplicates_and_bad_airports() -> Result<()> {
        let world = data()?.world;
        let mut twice = definition()?;
        twice.airports.push(sample()?);
        let error = GroundNetwork::from_definition(twice, &world)
            .err()
            .context("a duplicate must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::InvalidGround {
                airport: "EDDF".to_owned(),
                reason: "the airport is listed twice"
            })
        );
        let mut bad = definition()?;
        bad.airports[0].edges[0].from = 99;
        let error = GroundNetwork::from_definition(bad, &world)
            .err()
            .context("a bad layout must be rejected")?;
        assert!(format!("{error:#}").contains("invalid ground layout for EDDF"));
        Ok(())
    }

    #[test]
    fn test_shipped_ground_file_loads() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        assert_eq!(network.attribution(), "© OpenStreetMap contributors");
        // Every major German airport has a layout with stands, holding
        // positions and exits.
        for ident in ["EDDF", "EDDM", "EDDB", "EDDL", "EDDH", "EDDK", "EDDS"] {
            let airport = world.airport_by_ident(ident.parse()?).context("airport")?;
            let ground = network
                .airport(airport)
                .with_context(|| format!("{ident} layout"))?;
            assert!(
                ground.stands().len() >= 100,
                "{ident}: {}",
                ground.stands().len()
            );
            assert_ne!(ground.holding_positions().len(), 0, "{ident}");
            assert_ne!(ground.exits().len(), 0, "{ident}");
        }
        assert!(network.airports().len() >= 20);
        Ok(())
    }
}
