//! The static world: airports and runways.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::{CoreError, Id};

mod airport;
pub mod codes;
mod definition;
mod designator;
mod runway;

pub use airport::{Airport, AirportDefinition, AirportKind};
pub use definition::{FORMAT_VERSION, WorldDefinition};
pub use designator::{RunwayDesignator, RunwaySide};
pub use runway::{GeometrySource, Runway, RunwayDefinition, RunwayEnd, Surface};

/// A validated set of airports and runways with lookup indices.
///
/// Runways are stored grouped by airport, so [`World::runways_of`] is a slice.
#[derive(Debug, Clone, PartialEq)]
pub struct World {
    airports: Vec<Airport>,
    runways: Vec<Runway>,
    /// `runway_starts[i]..runway_starts[i + 1]` are airport `i`'s runways.
    runway_starts: Vec<usize>,
    by_ident: HashMap<codes::AirportIdent, Id<Airport>>,
    by_icao: HashMap<codes::IcaoCode, Id<Airport>>,
}

impl World {
    /// A world without airports.
    pub fn empty() -> Self {
        Self {
            airports: Vec::new(),
            runways: Vec::new(),
            runway_starts: vec![0],
            by_ident: HashMap::new(),
            by_icao: HashMap::new(),
        }
    }

    /// Validates every airport and runway and resolves runway references.
    pub fn from_definition(definition: WorldDefinition) -> Result<Self> {
        let mut airports = Vec::with_capacity(definition.airports.len());
        let mut by_ident = HashMap::with_capacity(definition.airports.len());
        let mut by_icao = HashMap::with_capacity(definition.airports.len());
        for airport in definition.airports {
            let ident = airport.ident;
            let airport = Airport::new(airport)?;
            let id = Id::from_usize(airports.len()).context("too many airports")?;
            if by_ident.insert(ident, id).is_some() {
                return Err(CoreError::DuplicateAirport {
                    ident: ident.to_string(),
                }
                .into());
            }
            if let Some(icao) = airport.icao() {
                by_icao.insert(icao, id);
            }
            airports.push(airport);
        }

        let mut runways = Vec::with_capacity(definition.runways.len());
        for runway in &definition.runways {
            let airport =
                *by_ident
                    .get(&runway.airport)
                    .ok_or_else(|| CoreError::UnknownAirport {
                        ident: runway.airport.to_string(),
                    })?;
            runways.push(Runway::new(airport, runway)?);
        }
        // Stable, so runways keep their file order within an airport.
        runways.sort_by_key(Runway::airport);

        let mut runway_starts = Vec::with_capacity(airports.len() + 1);
        let mut next = 0;
        for index in 0..airports.len() {
            runway_starts.push(next);
            while runways
                .get(next)
                .is_some_and(|runway| runway.airport().as_usize() == index)
            {
                next += 1;
            }
        }
        runway_starts.push(next);

        Ok(Self {
            airports,
            runways,
            runway_starts,
            by_ident,
            by_icao,
        })
    }

    /// Parses and validates a world file.
    pub fn from_json(text: &str) -> Result<Self> {
        Self::from_definition(WorldDefinition::from_json(text)?).context("invalid world file")
    }

    /// The world as a definition again, e.g. for writing a file.
    pub fn to_definition(&self) -> WorldDefinition {
        let mut runways = Vec::with_capacity(self.runways.len());
        for runway in &self.runways {
            let airport = self.airports[runway.airport().as_usize()].ident();
            runways.push(RunwayDefinition {
                airport,
                ends: *runway.ends(),
                length: runway.length(),
                width: runway.width(),
                surface: runway.surface(),
                geometry: runway.geometry(),
            });
        }
        WorldDefinition {
            format_version: FORMAT_VERSION,
            airports: self
                .airports
                .iter()
                .map(|a| a.definition().clone())
                .collect(),
            runways,
        }
    }

    /// Number of airports.
    pub fn airport_count(&self) -> usize {
        self.airports.len()
    }

    /// Number of runways.
    pub fn runway_count(&self) -> usize {
        self.runways.len()
    }

    /// The airport with `id`, if it exists.
    pub fn airport(&self, id: Id<Airport>) -> Option<&Airport> {
        self.airports.get(id.as_usize())
    }

    /// All airports with their ids, in file order.
    pub fn airports(&self) -> impl ExactSizeIterator<Item = (Id<Airport>, &Airport)> {
        // SAFETY: construction checked every index fits in an `Id`.
        #[allow(clippy::cast_possible_truncation)]
        self.airports
            .iter()
            .enumerate()
            .map(|(index, airport)| (Id::new(index as u32), airport))
    }

    /// Looks up an airport by ident.
    pub fn airport_by_ident(&self, ident: codes::AirportIdent) -> Option<Id<Airport>> {
        self.by_ident.get(&ident).copied()
    }

    /// Looks up an airport by ICAO code.
    pub fn airport_by_icao(&self, icao: codes::IcaoCode) -> Option<Id<Airport>> {
        self.by_icao.get(&icao).copied()
    }

    /// The runways of `airport`; empty for unknown ids.
    pub fn runways_of(&self, airport: Id<Airport>) -> &[Runway] {
        let index = airport.as_usize();
        match (
            self.runway_starts.get(index),
            self.runway_starts.get(index + 1),
        ) {
            (Some(&start), Some(&end)) => &self.runways[start..end],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        CoreError, LatLon,
        units::{Degrees, Feet, Meters},
        world::{airport::tests::frankfurt, runway::tests::frankfurt_07c},
    };

    fn munich() -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: "EDDM".parse()?,
            icao: Some("EDDM".parse()?),
            iata: Some("MUC".parse()?),
            name: "Munich Airport".to_owned(),
            location: LatLon::new(48.353_783, 11.786_086)?,
            elevation: Feet::new(1487.0)?,
            kind: AirportKind::Major,
            country: "DE".parse()?,
            catchment: None,
        })
    }

    fn airfield(ident: &str) -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: ident.parse()?,
            icao: None,
            iata: None,
            name: format!("Airfield {ident}"),
            location: LatLon::new(51.0, 10.0)?,
            elevation: Feet::new(500.0)?,
            kind: AirportKind::Airfield,
            country: "DE".parse()?,
            catchment: None,
        })
    }

    fn definition() -> Result<WorldDefinition> {
        Ok(WorldDefinition {
            format_version: FORMAT_VERSION,
            airports: vec![frankfurt()?, munich()?, airfield("DE-0062")?],
            // Deliberately interleaved: runways need not be grouped in the
            // file.
            runways: vec![
                frankfurt_07c("EDDF")?,
                frankfurt_07c("EDDM")?,
                frankfurt_07c("EDDF")?,
            ],
        })
    }

    fn rejection(definition: WorldDefinition) -> Result<CoreError> {
        let error = World::from_definition(definition)
            .err()
            .context("must be rejected")?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    #[test]
    fn test_world_empty_equals_empty_definition() -> Result<()> {
        let empty = WorldDefinition {
            format_version: FORMAT_VERSION,
            airports: Vec::new(),
            runways: Vec::new(),
        };
        assert_eq!(World::from_definition(empty)?, World::empty());
        Ok(())
    }

    #[test]
    fn test_world_empty_has_nothing() {
        let world = World::empty();
        assert_eq!(world.airport_count(), 0);
        assert_eq!(world.runway_count(), 0);
        assert!(world.airport(Id::new(0)).is_none());
        assert_eq!(world.runways_of(Id::new(0)).len(), 0);
    }

    #[test]
    fn test_world_from_definition_builds_lookups() -> Result<()> {
        let world = World::from_definition(definition()?)?;
        assert_eq!(world.airport_count(), 3);
        assert_eq!(world.runway_count(), 3);
        let eddf = world
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF by ident")?;
        assert_eq!(world.airport_by_icao("EDDF".parse()?), Some(eddf));
        assert_eq!(
            world.airport(eddf).map(Airport::name),
            Some("Frankfurt am Main Airport")
        );
        let local = world
            .airport_by_ident("DE-0062".parse()?)
            .context("local ident")?;
        assert!(world.airport(local).is_some());
        assert!(world.airport_by_icao("EDDH".parse()?).is_none());
        Ok(())
    }

    #[test]
    fn test_world_groups_runways_by_airport() -> Result<()> {
        let world = World::from_definition(definition()?)?;
        let frankfurt_id = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let munich_id = world.airport_by_ident("EDDM".parse()?).context("EDDM")?;
        let local = world
            .airport_by_ident("DE-0062".parse()?)
            .context("local")?;
        assert_eq!(world.runways_of(frankfurt_id).len(), 2);
        assert_eq!(world.runways_of(munich_id).len(), 1);
        assert_eq!(world.runways_of(local).len(), 0);
        assert!(
            world
                .runways_of(frankfurt_id)
                .iter()
                .all(|runway| runway.airport() == frankfurt_id)
        );
        Ok(())
    }

    #[test]
    fn test_world_iterates_airports_with_ids() -> Result<()> {
        let world = World::from_definition(definition()?)?;
        for (id, airport) in world.airports() {
            assert_eq!(world.airport_by_ident(airport.ident()), Some(id));
        }
        assert_eq!(world.airports().count(), 3);
        Ok(())
    }

    #[test]
    fn test_world_rejects_duplicate_ident() -> Result<()> {
        let mut definition = definition()?;
        definition.airports.push(frankfurt()?);
        assert_eq!(
            rejection(definition)?,
            CoreError::DuplicateAirport {
                ident: "EDDF".to_owned()
            }
        );
        Ok(())
    }

    #[test]
    fn test_world_rejects_runway_at_unknown_airport() -> Result<()> {
        let mut definition = definition()?;
        definition.runways.push(frankfurt_07c("EDDH")?);
        assert_eq!(
            rejection(definition)?,
            CoreError::UnknownAirport {
                ident: "EDDH".to_owned()
            }
        );
        Ok(())
    }

    #[test]
    fn test_world_rejects_invalid_airport_and_runway() -> Result<()> {
        let mut bad_airport = definition()?;
        bad_airport.airports[1].name = String::new();
        assert!(matches!(
            rejection(bad_airport)?,
            CoreError::InvalidAirport { .. }
        ));
        let mut bad_runway = definition()?;
        bad_runway.runways[0].length = Meters::new(-1.0)?;
        assert!(matches!(
            rejection(bad_runway)?,
            CoreError::InvalidRunway { .. }
        ));
        Ok(())
    }

    #[test]
    fn test_world_to_definition_round_trip() -> Result<()> {
        let world = World::from_definition(definition()?)?;
        let rebuilt = World::from_definition(world.to_definition())?;
        assert_eq!(rebuilt, world);
        Ok(())
    }

    #[test]
    fn test_world_from_json() -> Result<()> {
        let json = definition()?.to_json_pretty()?;
        assert_eq!(World::from_json(&json)?.airport_count(), 3);
        Ok(())
    }

    /// A world of `airports` airfields; runway `i` goes to airport `owners[i] %
    /// airports`.
    fn generated(airports: usize, owners: &[usize], headings: &[f64]) -> Result<WorldDefinition> {
        let mut definition = WorldDefinition {
            format_version: FORMAT_VERSION,
            airports: Vec::with_capacity(airports),
            runways: Vec::with_capacity(owners.len()),
        };
        for index in 0..airports {
            definition
                .airports
                .push(airfield(&format!("DE-{index:04}"))?);
        }
        for (owner, heading) in owners.iter().zip(headings) {
            let mut runway = frankfurt_07c(&format!("DE-{:04}", owner % airports))?;
            let start = runway.ends[0].threshold();
            let heading = Degrees::new(*heading)?;
            let end = start.destination(heading, runway.length);
            let [first, second] = runway.ends;
            runway.ends = [
                RunwayEnd::new(first.designator(), start, heading, Meters::new(0.0)?)?,
                RunwayEnd::new(
                    second.designator(),
                    end,
                    Degrees::new(heading.value() + 180.0)?,
                    Meters::new(0.0)?,
                )?,
            ];
            definition.runways.push(runway);
        }
        Ok(definition)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_world_valid_definitions_partition_runways(
            airports in 1_usize..20,
            runways in prop::collection::vec((0_usize..100, 0.0_f64..360.0), 0..40),
        ) {
            let (owners, headings): (Vec<_>, Vec<_>) = runways.into_iter().unzip();
            let definition = generated(airports, &owners, &headings).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let world = World::from_definition(definition).map_err(|e| TestCaseError::fail(format!("{e:#}")))?;
            prop_assert_eq!(world.airport_count(), airports);
            let grouped: usize = world.airports().map(|(id, _)| world.runways_of(id).len()).sum();
            prop_assert_eq!(grouped, owners.len());
            for (id, _) in world.airports() {
                prop_assert!(world.runways_of(id).iter().all(|runway| runway.airport() == id));
            }
        }

        #[test]
        fn test_world_corrupted_references_are_rejected(
            airports in 1_usize..20,
            owners in prop::collection::vec(0_usize..100, 1..20),
            victim in 0_usize..20,
            duplicate in any::<bool>(),
        ) {
            let headings = vec![90.0; owners.len()];
            let mut definition = generated(airports, &owners, &headings).map_err(|e| TestCaseError::fail(e.to_string()))?;
            if duplicate {
                let copy = definition.airports[victim % airports].clone();
                definition.airports.push(copy);
            } else {
                let index = victim % definition.runways.len();
                definition.runways[index].airport = "DE-9999".parse().map_err(|e: anyhow::Error| TestCaseError::fail(e.to_string()))?;
            }
            prop_assert!(World::from_definition(definition).is_err());
        }
    }

    /// The generated world file shipped with the project (see
    /// docs/data-sources.md).
    const GERMANY: &str = include_str!("../../../../assets/world/germany.world.json");

    #[test]
    fn test_shipped_germany_world_is_valid() -> Result<()> {
        let world = World::from_json(GERMANY).context("assets/world/germany.world.json")?;
        assert!(
            world.airport_count() > 400,
            "{} airports",
            world.airport_count()
        );
        let frankfurt = world.airport_by_icao("EDDF".parse()?).context("EDDF")?;
        assert_eq!(world.runways_of(frankfurt).len(), 4);
        let majors = world
            .airports()
            .filter(|(_, a)| a.kind() == AirportKind::Major)
            .count();
        assert!(majors >= 20, "{majors} major airports");
        Ok(())
    }
}
