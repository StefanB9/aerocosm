//! Airlines: the hand-curated catalog of carriers that fly the schedules (plans
//! D32, D38, D39).

use std::{collections::HashMap, fmt, str::FromStr};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use crate::world::codes::{AirlineIata, AirlineIcao};
use crate::{
    AircraftCatalog, CoreError, Id,
    world::{
        World,
        codes::{AircraftTypeCode, AirportIdent},
    },
};

/// Airline catalog file format version this build reads and writes.
pub const AIRLINE_FORMAT_VERSION: u32 = 1;

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

/// A brand colour such as `#0A1D3D`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour([u8; 3]);

impl Colour {
    /// Red, green and blue bytes.
    pub const fn rgb(self) -> [u8; 3] {
        self.0
    }
}

impl FromStr for Colour {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let invalid = || CoreError::InvalidCode {
            kind: "colour",
            input: input.to_owned(),
        };
        let hex = input.strip_prefix('#').ok_or_else(invalid)?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid().into());
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).map_err(|_| invalid());
        Ok(Self([channel(0)?, channel(2)?, channel(4)?]))
    }
}

impl fmt::Display for Colour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02X}{g:02X}{b:02X}")
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <&str>::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// An airport an airline operates from, with its relative share of the
/// airline's flying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base {
    /// The airport.
    pub airport: AirportIdent,
    /// Relative weight among the airline's bases.
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

    use super::*;
    use crate::CoreError;

    fn lufthansa() -> Result<AirlineDefinition> {
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
    fn test_airline_codes_and_colour_parse() -> Result<()> {
        assert_eq!("DLH".parse::<AirlineIcao>()?.as_str(), "DLH");
        assert!("DL".parse::<AirlineIcao>().is_err());
        assert_eq!("U2".parse::<AirlineIata>()?.as_str(), "U2");
        assert!("U".parse::<AirlineIata>().is_err());
        let colour: Colour = "#0A1D3D".parse()?;
        assert_eq!(colour.rgb(), [0x0A, 0x1D, 0x3D]);
        assert_eq!(colour.to_string(), "#0A1D3D");
        for bad in ["0A1D3D", "#0A1D3", "#GG0000", "#0A1D3D0"] {
            assert!(bad.parse::<Colour>().is_err(), "{bad}");
        }
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
            AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?;
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
}
