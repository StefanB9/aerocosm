//! The validated set of airlines.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{Airline, AirlineDefinition, AirlineIcao};
use crate::{AircraftCatalog, CoreError, Id, world::World};

/// Airline catalog file format version this build reads and writes.
pub const AIRLINE_FORMAT_VERSION: u32 = 1;

/// The contents of an airline catalog file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirlineCatalogDefinition {
    /// Must equal [`AIRLINE_FORMAT_VERSION`].
    pub format_version: u32,
    /// All airlines.
    pub airlines: Vec<AirlineDefinition>,
}

impl AirlineCatalogDefinition {
    /// Serialises as indented JSON with a trailing newline.
    pub fn to_json_pretty(&self) -> Result<String> {
        let mut json =
            serde_json::to_string_pretty(self).context("cannot serialise airline catalog")?;
        json.push('\n');
        Ok(json)
    }
}

/// The validated set of airlines, indexed by ICAO designator.
#[derive(Debug, Clone, PartialEq)]
pub struct AirlineCatalog {
    airlines: Vec<Airline>,
    by_icao: HashMap<AirlineIcao, Id<Airline>>,
}

impl AirlineCatalog {
    /// Validates every airline and rejects duplicate designators.
    pub fn from_definition(definition: AirlineCatalogDefinition) -> Result<Self> {
        if definition.format_version != AIRLINE_FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "airline catalog",
                found: definition.format_version,
                supported: AIRLINE_FORMAT_VERSION,
            }
            .into());
        }
        let mut airlines = Vec::with_capacity(definition.airlines.len());
        let mut by_icao = HashMap::with_capacity(definition.airlines.len());
        for airline in definition.airlines {
            let airline = Airline::new(airline)?;
            let id = Id::from_usize(airlines.len()).context("too many airlines")?;
            if by_icao.insert(airline.icao(), id).is_some() {
                return Err(CoreError::DuplicateAirline {
                    icao: airline.icao().to_string(),
                }
                .into());
            }
            airlines.push(airline);
        }
        Ok(Self { airlines, by_icao })
    }

    /// Parses and validates a catalog file.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition = serde_json::from_str(text).context("malformed airline catalog")?;
        Self::from_definition(definition).context("invalid airline catalog")
    }

    /// The catalog as a definition again.
    pub fn to_definition(&self) -> AirlineCatalogDefinition {
        AirlineCatalogDefinition {
            format_version: AIRLINE_FORMAT_VERSION,
            airlines: self.airlines.iter().map(|a| a.definition.clone()).collect(),
        }
    }

    /// Checks that every base exists in `world` and every fleet type in
    /// `aircraft`.
    pub fn check_references(&self, world: &World, aircraft: &AircraftCatalog) -> Result<()> {
        for airline in &self.airlines {
            for base in airline.bases() {
                if world.airport_by_ident(base.airport).is_none() {
                    return Err(CoreError::UnknownAirport {
                        ident: base.airport.to_string(),
                    })
                    .with_context(|| format!("base of airline {}", airline.icao()));
                }
            }
            for share in airline.fleet() {
                if aircraft.by_code(share.aircraft).is_none() {
                    return Err(CoreError::UnknownAircraftType {
                        code: share.aircraft.to_string(),
                    })
                    .with_context(|| format!("fleet of airline {}", airline.icao()));
                }
            }
        }
        Ok(())
    }

    /// Number of airlines.
    pub fn len(&self) -> usize {
        self.airlines.len()
    }

    /// Whether the catalog has no airlines.
    pub fn is_empty(&self) -> bool {
        self.airlines.is_empty()
    }

    /// Looks up an airline by ICAO designator.
    pub fn by_icao(&self, icao: AirlineIcao) -> Option<Id<Airline>> {
        self.by_icao.get(&icao).copied()
    }

    /// The airline with `id`, if it exists.
    pub fn get(&self, id: Id<Airline>) -> Option<&Airline> {
        self.airlines.get(id.as_usize())
    }

    /// All airlines in file order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Airline> {
        self.airlines.iter()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::{super::tests::lufthansa, *};

    #[test]
    fn test_catalog_lookup_and_duplicates() -> Result<()> {
        let catalog = AirlineCatalog::from_definition(AirlineCatalogDefinition {
            format_version: AIRLINE_FORMAT_VERSION,
            airlines: vec![lufthansa()?],
        })?;
        assert_eq!(catalog.len(), 1);
        let id = catalog.by_icao("DLH".parse()?).context("DLH")?;
        assert_eq!(catalog.get(id).map(Airline::name), Some("Lufthansa"));
        assert_eq!(catalog.iter().count(), 1);

        let error = AirlineCatalog::from_definition(AirlineCatalogDefinition {
            format_version: AIRLINE_FORMAT_VERSION,
            airlines: vec![lufthansa()?, lufthansa()?],
        })
        .err()
        .context("duplicates must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::DuplicateAirline {
                icao: "DLH".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn test_catalog_check_references() -> Result<()> {
        let aircraft =
            AircraftCatalog::from_json(include_str!("../../../../assets/aircraft/types.json"))?;
        let world = World::from_json(
            r#"{"format_version": 2, "runways": [], "airports": [
                {"ident": "EDDF", "name": "Frankfurt", "location": {"lat": 50.03, "lon": 8.57},
                 "elevation": 364.0, "kind": "major", "country": "DE"}]}"#,
        )?;
        let catalog = |definition| {
            AirlineCatalog::from_definition(AirlineCatalogDefinition {
                format_version: AIRLINE_FORMAT_VERSION,
                airlines: vec![definition],
            })
        };
        let mut ok = lufthansa()?;
        ok.bases.truncate(1);
        catalog(ok)?.check_references(&world, &aircraft)?;
        let error = catalog(lufthansa()?)?
            .check_references(&world, &aircraft)
            .err()
            .context("EDDM unknown")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnknownAirport {
                ident: "EDDM".to_owned()
            })
        );
        let mut exotic = lufthansa()?;
        exotic.bases.truncate(1);
        exotic.fleet[1].aircraft = "C172".parse()?;
        let error = catalog(exotic)?
            .check_references(&world, &aircraft)
            .err()
            .context("C172 unknown")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnknownAircraftType {
                code: "C172".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn test_catalog_rejects_unknown_version() -> Result<()> {
        let error = AirlineCatalog::from_json(r#"{"format_version": 9, "airlines": []}"#)
            .err()
            .context("must fail")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "airline catalog",
                found: 9,
                supported: AIRLINE_FORMAT_VERSION
            })
        );
        Ok(())
    }

    #[test]
    fn test_catalog_json_round_trip() -> Result<()> {
        let definition = AirlineCatalogDefinition {
            format_version: AIRLINE_FORMAT_VERSION,
            airlines: vec![lufthansa()?],
        };
        let json = definition.to_json_pretty()?;
        assert!(json.contains("\"network\": \"hub\""), "{json}");
        let catalog = AirlineCatalog::from_json(&json)?;
        assert_eq!(catalog.to_definition(), definition);
        Ok(())
    }

    #[test]
    fn test_shipped_airline_catalog_is_valid_and_resolves() -> Result<()> {
        let airlines =
            AirlineCatalog::from_json(include_str!("../../../../assets/airlines/airlines.json"))
                .context("assets/airlines/airlines.json")?;
        let world = World::from_json(include_str!("../../../../assets/world/germany.world.json"))?;
        let aircraft =
            AircraftCatalog::from_json(include_str!("../../../../assets/aircraft/types.json"))?;
        airlines.check_references(&world, &aircraft)?;
        assert_eq!(airlines.len(), 23);
        let cargo = airlines
            .iter()
            .filter(|a| a.kind() == super::super::AirlineKind::Cargo)
            .count();
        assert_eq!(cargo, 4);
        for airline in airlines
            .iter()
            .filter(|a| a.kind() == super::super::AirlineKind::Cargo)
        {
            for share in airline.fleet() {
                let id = aircraft.by_code(share.aircraft).context("type")?;
                assert!(
                    aircraft
                        .get(id)
                        .is_some_and(crate::AircraftType::is_freighter),
                    "{} flies {}",
                    airline.icao(),
                    share.aircraft
                );
            }
        }
        Ok(())
    }
}
