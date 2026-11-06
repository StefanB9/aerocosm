//! Airlines: the hand-curated catalog of carriers that fly the schedules (plans
//! D32, D38, D39).

use anyhow::Result;
use serde::{Deserialize, Serialize};

mod catalog;
mod colour;

pub use self::{
    catalog::{AIRLINE_FORMAT_VERSION, AirlineCatalog, AirlineCatalogDefinition},
    colour::Colour,
};
pub use crate::world::codes::{AirlineIata, AirlineIcao};
use crate::{
    CoreError,
    world::codes::{AircraftTypeCode, AirportIdent},
};

/// Fleet shares may deviate this much from summing to 1 (rounding in curated
/// data).
const SHARE_TOLERANCE: f64 = 0.01;

/// What an airline carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirlineKind {
    /// Passengers; flies by day.
    Passenger,
    /// Freight; flies mostly at night (D39).
    Cargo,
}

/// How an airline builds its network, which drives route assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    /// Hub-and-spoke from its bases, with connecting banks.
    Hub,
    /// Point-to-point from its bases.
    PointToPoint,
    /// A foreign carrier flying into Germany from its home hubs (edge
    /// airports).
    ForeignHub,
}

/// An airport an airline operates from, with its estimated market share there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base {
    /// The airport.
    pub airport: AirportIdent,
    /// Estimated share of the airport's departures flown by this airline (passenger or
    /// cargo traffic respectively). Schedule generation splits a route between the airlines
    /// based at either end in proportion to these weights.
    pub weight: f64,
}

/// One aircraft type's share of an airline's fleet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetShare {
    /// The aircraft type.
    pub aircraft: AircraftTypeCode,
    /// Fraction of the fleet, `0..=1`; an airline's shares sum to 1.
    pub share: f64,
}

/// Unvalidated airline data as stored in the catalog file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirlineDefinition {
    /// ICAO designator, the callsign prefix (e.g. `DLH`).
    pub icao: AirlineIcao,
    /// IATA designator, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iata: Option<AirlineIata>,
    /// Name.
    pub name: String,
    /// Radio telephony callsign, e.g. `LUFTHANSA`.
    pub callsign: String,
    /// Passenger or cargo.
    pub kind: AirlineKind,
    /// Network style.
    pub network: Network,
    /// Bases with weights.
    pub bases: Vec<Base>,
    /// Fleet type mix (D38: tail counts are derived by schedule generation).
    pub fleet: Vec<FleetShare>,
    /// Brand colour.
    pub colour: Colour,
    /// Where the figures come from.
    pub source: String,
}

/// A validated airline.
#[derive(Debug, Clone, PartialEq)]
pub struct Airline {
    definition: AirlineDefinition,
}

impl Airline {
    /// Validates names, bases (distinct, positive weights) and fleet shares
    /// (positive, summing to 1).
    pub fn new(definition: AirlineDefinition) -> Result<Self> {
        let invalid = |reason| CoreError::InvalidAirline {
            icao: definition.icao.to_string(),
            reason,
        };
        let d = &definition;
        if d.name.is_empty() || d.callsign.is_empty() || d.source.is_empty() {
            return Err(invalid("name, callsign and source must be non-empty").into());
        }
        if d.bases.is_empty()
            || !d
                .bases
                .iter()
                .all(|b| b.weight.is_finite() && b.weight > 0.0)
        {
            return Err(invalid("bases must be non-empty with positive weights").into());
        }
        for (index, base) in d.bases.iter().enumerate() {
            if d.bases[..index]
                .iter()
                .any(|other| other.airport == base.airport)
            {
                return Err(invalid("bases must be distinct").into());
            }
        }
        let total: f64 = d.fleet.iter().map(|f| f.share).sum();
        if d.fleet.is_empty()
            || !d.fleet.iter().all(|f| f.share.is_finite() && f.share > 0.0)
            || (total - 1.0).abs() > SHARE_TOLERANCE
        {
            return Err(invalid("fleet shares must be positive and sum to 1").into());
        }
        Ok(Self { definition })
    }

    /// ICAO designator.
    pub const fn icao(&self) -> AirlineIcao {
        self.definition.icao
    }

    /// IATA designator, if any.
    pub const fn iata(&self) -> Option<AirlineIata> {
        self.definition.iata
    }

    /// Name.
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// Radio telephony callsign.
    pub fn callsign(&self) -> &str {
        &self.definition.callsign
    }

    /// Passenger or cargo.
    pub const fn kind(&self) -> AirlineKind {
        self.definition.kind
    }

    /// Network style.
    pub const fn network(&self) -> Network {
        self.definition.network
    }

    /// Bases with weights.
    pub fn bases(&self) -> &[Base] {
        &self.definition.bases
    }

    /// Fleet type mix.
    pub fn fleet(&self) -> &[FleetShare] {
        &self.definition.fleet
    }

    /// Brand colour.
    pub const fn colour(&self) -> Colour {
        self.definition.colour
    }

    /// Where the figures come from.
    pub fn source(&self) -> &str {
        &self.definition.source
    }
}

#[cfg(test)]
pub(super) mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    pub(super) fn lufthansa() -> Result<AirlineDefinition> {
        Ok(AirlineDefinition {
            icao: "DLH".parse()?,
            iata: Some("LH".parse()?),
            name: "Lufthansa".to_owned(),
            callsign: "LUFTHANSA".to_owned(),
            kind: AirlineKind::Passenger,
            network: Network::Hub,
            bases: vec![
                Base {
                    airport: "EDDF".parse()?,
                    weight: 0.6,
                },
                Base {
                    airport: "EDDM".parse()?,
                    weight: 0.4,
                },
            ],
            fleet: vec![
                FleetShare {
                    aircraft: "A20N".parse()?,
                    share: 0.7,
                },
                FleetShare {
                    aircraft: "A359".parse()?,
                    share: 0.3,
                },
            ],
            colour: "#0A1D3D".parse()?,
            source: "test fixture".to_owned(),
        })
    }

    fn reason(definition: AirlineDefinition) -> Result<&'static str> {
        let error = Airline::new(definition).err().context("must be rejected")?;
        match error
            .downcast::<CoreError>()
            .context("expected a CoreError")?
        {
            CoreError::InvalidAirline { reason, .. } => Ok(reason),
            other => anyhow::bail!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_airline_codes_parse() -> Result<()> {
        assert_eq!("DLH".parse::<AirlineIcao>()?.as_str(), "DLH");
        assert!("DL".parse::<AirlineIcao>().is_err());
        assert_eq!("U2".parse::<AirlineIata>()?.as_str(), "U2");
        assert!("U".parse::<AirlineIata>().is_err());
        Ok(())
    }

    #[test]
    fn test_airline_new_exposes_fields() -> Result<()> {
        let airline = Airline::new(lufthansa()?)?;
        assert_eq!(airline.icao().as_str(), "DLH");
        assert_eq!(airline.iata().map(|c| c.to_string()).as_deref(), Some("LH"));
        assert_eq!(airline.name(), "Lufthansa");
        assert_eq!(airline.callsign(), "LUFTHANSA");
        assert_eq!(airline.kind(), AirlineKind::Passenger);
        assert_eq!(airline.network(), Network::Hub);
        assert_eq!(airline.bases().len(), 2);
        assert_eq!(airline.fleet().len(), 2);
        assert_eq!(airline.colour().rgb(), [0x0A, 0x1D, 0x3D]);
        assert_eq!(airline.source(), "test fixture");
        Ok(())
    }

    #[test]
    fn test_airline_new_rejects_inconsistent_definitions() -> Result<()> {
        let mut unnamed = lufthansa()?;
        unnamed.callsign = String::new();
        assert_eq!(
            reason(unnamed)?,
            "name, callsign and source must be non-empty"
        );

        let mut baseless = lufthansa()?;
        baseless.bases.clear();
        assert_eq!(
            reason(baseless)?,
            "bases must be non-empty with positive weights"
        );

        let mut negative = lufthansa()?;
        negative.bases[0].weight = -1.0;
        assert_eq!(
            reason(negative)?,
            "bases must be non-empty with positive weights"
        );

        let mut repeated = lufthansa()?;
        repeated.bases[1].airport = "EDDF".parse()?;
        assert_eq!(reason(repeated)?, "bases must be distinct");

        let mut lopsided = lufthansa()?;
        lopsided.fleet[0].share = 0.5;
        assert_eq!(
            reason(lopsided)?,
            "fleet shares must be positive and sum to 1"
        );

        let mut empty = lufthansa()?;
        empty.fleet.clear();
        assert_eq!(reason(empty)?, "fleet shares must be positive and sum to 1");
        Ok(())
    }
}
