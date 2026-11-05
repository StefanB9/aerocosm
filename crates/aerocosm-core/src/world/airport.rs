//! Aerodromes.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::codes::{AirportIdent, CountryCode, IataCode, IcaoCode};
use crate::{CoreError, LatLon, units::Feet};

/// The role an aerodrome plays in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirportKind {
    /// A large commercial airport (`OurAirports` `large_airport`).
    Major,
    /// A smaller airport with scheduled service or significant traffic.
    Regional,
    /// A general-aviation airfield without scheduled service.
    Airfield,
    /// A foreign airport where traffic leaving the simulated region ends (M2).
    Edge,
}

/// Unvalidated airport data as stored in the world file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirportDefinition {
    /// Unique identifier: the ICAO code where there is one.
    pub ident: AirportIdent,
    /// ICAO location indicator; equals `ident` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icao: Option<IcaoCode>,
    /// IATA code, if assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iata: Option<IataCode>,
    /// Human-readable name.
    pub name: String,
    /// Aerodrome reference point.
    pub location: LatLon,
    /// Elevation above mean sea level.
    pub elevation: Feet,
    /// Role in the simulation.
    pub kind: AirportKind,
    /// Country the aerodrome is in.
    pub country: CountryCode,
}

/// A validated aerodrome.
#[derive(Debug, Clone, PartialEq)]
pub struct Airport {
    definition: AirportDefinition,
}

impl Airport {
    /// Validates a definition: a trimmed, non-empty name, and an ICAO code (if
    /// any) equal to the ident so either lookup finds the same airport.
    pub fn new(definition: AirportDefinition) -> Result<Self> {
        let invalid = |reason| CoreError::InvalidAirport {
            ident: definition.ident.to_string(),
            reason,
        };
        let name = definition.name.as_str();
        if name.is_empty() || name.trim() != name {
            return Err(invalid("name must be non-empty without surrounding whitespace").into());
        }
        if let Some(icao) = definition.icao
            && AirportIdent::from(icao) != definition.ident
        {
            return Err(invalid("ICAO code must equal the ident").into());
        }
        Ok(Self { definition })
    }

    /// Unique identifier.
    pub const fn ident(&self) -> AirportIdent {
        self.definition.ident
    }

    /// ICAO location indicator, if the aerodrome has one.
    pub const fn icao(&self) -> Option<IcaoCode> {
        self.definition.icao
    }

    /// IATA code, if assigned.
    pub const fn iata(&self) -> Option<IataCode> {
        self.definition.iata
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// Aerodrome reference point.
    pub const fn location(&self) -> LatLon {
        self.definition.location
    }

    /// Elevation above mean sea level.
    pub const fn elevation(&self) -> Feet {
        self.definition.elevation
    }

    /// Role in the simulation.
    pub const fn kind(&self) -> AirportKind {
        self.definition.kind
    }

    /// Country the aerodrome is in.
    pub const fn country(&self) -> CountryCode {
        self.definition.country
    }

    /// The definition this airport was built from, for writing world files.
    pub const fn definition(&self) -> &AirportDefinition {
        &self.definition
    }
}

#[cfg(test)]
pub(super) mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    pub(in crate::world) fn frankfurt() -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: "EDDF".parse()?,
            icao: Some("EDDF".parse()?),
            iata: Some("FRA".parse()?),
            name: "Frankfurt am Main Airport".to_owned(),
            location: LatLon::new(50.033_333, 8.570_556)?,
            elevation: Feet::new(364.0)?,
            kind: AirportKind::Major,
            country: "DE".parse()?,
        })
    }

    fn rejection(definition: AirportDefinition) -> Result<CoreError> {
        let error = Airport::new(definition).err().context("must be rejected")?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    #[test]
    fn test_airport_new_exposes_fields() -> Result<()> {
        let airport = Airport::new(frankfurt()?)?;
        assert_eq!(airport.ident().as_str(), "EDDF");
        assert_eq!(
            airport.icao().map(|c| c.to_string()).as_deref(),
            Some("EDDF")
        );
        assert_eq!(
            airport.iata().map(|c| c.to_string()).as_deref(),
            Some("FRA")
        );
        assert_eq!(airport.name(), "Frankfurt am Main Airport");
        assert!((airport.location().latitude().value() - 50.033_333).abs() < 1e-9);
        assert!((airport.elevation().value() - 364.0).abs() < f64::EPSILON);
        assert_eq!(airport.kind(), AirportKind::Major);
        assert_eq!(airport.country().as_str(), "DE");
        Ok(())
    }

    #[test]
    fn test_airport_new_accepts_local_ident_without_icao() -> Result<()> {
        let mut definition = frankfurt()?;
        definition.ident = "DE-0062".parse()?;
        definition.icao = None;
        definition.iata = None;
        definition.kind = AirportKind::Airfield;
        assert!(Airport::new(definition)?.icao().is_none());
        Ok(())
    }

    #[test]
    fn test_airport_new_rejects_blank_name() -> Result<()> {
        for name in ["", "   ", " padded"] {
            let mut definition = frankfurt()?;
            definition.name = name.to_owned();
            assert_eq!(
                rejection(definition)?,
                CoreError::InvalidAirport {
                    ident: "EDDF".to_owned(),
                    reason: "name must be non-empty without surrounding whitespace",
                }
            );
        }
        Ok(())
    }

    #[test]
    fn test_airport_new_rejects_icao_differing_from_ident() -> Result<()> {
        let mut definition = frankfurt()?;
        definition.icao = Some("EDDM".parse()?);
        assert_eq!(
            rejection(definition)?,
            CoreError::InvalidAirport {
                ident: "EDDF".to_owned(),
                reason: "ICAO code must equal the ident",
            }
        );
        Ok(())
    }

    #[test]
    fn test_airport_definition_serde_round_trip() -> Result<()> {
        let definition = frankfurt()?;
        let json = serde_json::to_string(&definition).context("serialise")?;
        assert!(json.contains(r#""kind":"major""#), "{json}");
        let back: AirportDefinition = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back, definition);
        Ok(())
    }

    #[test]
    fn test_airport_definition_omits_absent_codes() -> Result<()> {
        let mut definition = frankfurt()?;
        definition.iata = None;
        let json = serde_json::to_string(&definition).context("serialise")?;
        assert!(!json.contains("iata"), "{json}");
        let back: AirportDefinition = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back.iata, None);
        Ok(())
    }
}
