//! Aircraft types and the hand-curated catalog of them.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    CoreError, Id,
    units::{Feet, FeetPerMinute, Knots, Meters, NauticalMiles},
    world::codes::AircraftTypeCode,
};

/// Aircraft catalog file format version this build reads and writes.
pub const CATALOG_FORMAT_VERSION: u32 = 1;

/// ICAO wake turbulence category, which drives separation minima.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeCategory {
    /// MTOW up to 7 t.
    Light,
    /// MTOW 7–136 t.
    Medium,
    /// MTOW above 136 t.
    Heavy,
    /// The A380.
    Super,
}

/// Unvalidated aircraft type data as stored in the catalog file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AircraftTypeDefinition {
    /// ICAO type designator.
    pub code: AircraftTypeCode,
    /// Marketing name and variant.
    pub name: String,
    /// Wake turbulence category.
    pub wake: WakeCategory,
    /// Typical seats in a European airline configuration.
    pub seats: u16,
    /// Typical cruise true airspeed.
    pub cruise_speed: Knots,
    /// Typical cruise altitude.
    pub cruise_altitude: Feet,
    /// Average climb rate to cruise.
    pub climb_rate: FeetPerMinute,
    /// Average descent rate from cruise.
    pub descent_rate: FeetPerMinute,
    /// Final approach speed.
    pub approach_speed: Knots,
    /// Typical still-air range with a full passenger load.
    pub range: NauticalMiles,
    /// Take-off field length at maximum take-off weight, sea level, ISA.
    pub min_runway_length: Meters,
    /// Where the figures come from.
    pub source: String,
}

/// A validated aircraft type.
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftType {
    definition: AircraftTypeDefinition,
}

impl AircraftType {
    /// Validates a definition for plausibility; the figures themselves are
    /// curated by hand.
    pub fn new(definition: AircraftTypeDefinition) -> Result<Self> {
        let invalid = |reason| CoreError::InvalidAircraftType {
            code: definition.code.to_string(),
            reason,
        };
        let d = &definition;
        if d.name.is_empty() || d.source.is_empty() {
            return Err(invalid("name and source must be non-empty").into());
        }
        if d.seats == 0 {
            return Err(invalid("seats must be positive").into());
        }
        if d.approach_speed.value() <= 0.0 || d.approach_speed >= d.cruise_speed {
            return Err(invalid("approach speed must be positive and below cruise speed").into());
        }
        if !(1000.0..=51_000.0).contains(&d.cruise_altitude.value()) {
            return Err(invalid("cruise altitude must be within 1000–51000 ft").into());
        }
        if d.climb_rate.value() <= 0.0 || d.descent_rate.value() <= 0.0 {
            return Err(invalid("climb and descent rates must be positive").into());
        }
        if d.range.value() <= 0.0 || d.min_runway_length.value() <= 0.0 {
            return Err(invalid("range and minimum runway length must be positive").into());
        }
        Ok(Self { definition })
    }

    /// ICAO type designator.
    pub const fn code(&self) -> AircraftTypeCode {
        self.definition.code
    }

    /// Marketing name and variant.
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// Wake turbulence category.
    pub const fn wake(&self) -> WakeCategory {
        self.definition.wake
    }

    /// Typical seats.
    pub const fn seats(&self) -> u16 {
        self.definition.seats
    }

    /// Typical cruise true airspeed.
    pub const fn cruise_speed(&self) -> Knots {
        self.definition.cruise_speed
    }

    /// Typical cruise altitude.
    pub const fn cruise_altitude(&self) -> Feet {
        self.definition.cruise_altitude
    }

    /// Average climb rate.
    pub const fn climb_rate(&self) -> FeetPerMinute {
        self.definition.climb_rate
    }

    /// Average descent rate.
    pub const fn descent_rate(&self) -> FeetPerMinute {
        self.definition.descent_rate
    }

    /// Final approach speed.
    pub const fn approach_speed(&self) -> Knots {
        self.definition.approach_speed
    }

    /// Typical range.
    pub const fn range(&self) -> NauticalMiles {
        self.definition.range
    }

    /// Take-off field length at maximum take-off weight.
    pub const fn min_runway_length(&self) -> Meters {
        self.definition.min_runway_length
    }

    /// Where the figures come from.
    pub fn source(&self) -> &str {
        &self.definition.source
    }
}

/// The contents of an aircraft catalog file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AircraftCatalogDefinition {
    /// Must equal [`CATALOG_FORMAT_VERSION`].
    pub format_version: u32,
    /// All types.
    pub types: Vec<AircraftTypeDefinition>,
}

impl AircraftCatalogDefinition {
    /// Serialises as indented JSON with a trailing newline.
    pub fn to_json_pretty(&self) -> Result<String> {
        let mut json = serde_json::to_string_pretty(self).context("cannot serialise catalog")?;
        json.push('\n');
        Ok(json)
    }
}

/// The validated set of aircraft types, indexed by designator.
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftCatalog {
    types: Vec<AircraftType>,
    by_code: HashMap<AircraftTypeCode, Id<AircraftType>>,
}

impl AircraftCatalog {
    /// Validates every type and rejects duplicate designators.
    pub fn from_definition(definition: AircraftCatalogDefinition) -> Result<Self> {
        if definition.format_version != CATALOG_FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "aircraft catalog",
                found: definition.format_version,
                supported: CATALOG_FORMAT_VERSION,
            }
            .into());
        }
        let mut types = Vec::with_capacity(definition.types.len());
        let mut by_code = HashMap::with_capacity(definition.types.len());
        for aircraft in definition.types {
            let aircraft = AircraftType::new(aircraft)?;
            let id = Id::from_usize(types.len()).context("too many aircraft types")?;
            if by_code.insert(aircraft.code(), id).is_some() {
                return Err(CoreError::DuplicateAircraftType {
                    code: aircraft.code().to_string(),
                }
                .into());
            }
            types.push(aircraft);
        }
        Ok(Self { types, by_code })
    }

    /// Parses and validates a catalog file.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition = serde_json::from_str(text).context("malformed aircraft catalog")?;
        Self::from_definition(definition).context("invalid aircraft catalog")
    }

    /// The catalog as a definition again.
    pub fn to_definition(&self) -> AircraftCatalogDefinition {
        AircraftCatalogDefinition {
            format_version: CATALOG_FORMAT_VERSION,
            types: self.types.iter().map(|t| t.definition.clone()).collect(),
        }
    }

    /// Number of types.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the catalog has no types.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Looks up a type by designator.
    pub fn by_code(&self, code: AircraftTypeCode) -> Option<Id<AircraftType>> {
        self.by_code.get(&code).copied()
    }

    /// The type with `id`, if it exists.
    pub fn get(&self, id: Id<AircraftType>) -> Option<&AircraftType> {
        self.types.get(id.as_usize())
    }

    /// All types in file order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &AircraftType> {
        self.types.iter()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    /// The real catalog shipped with the project.
    const CATALOG: &str = include_str!("../../../assets/aircraft/types.json");

    fn a320() -> Result<AircraftTypeDefinition> {
        Ok(AircraftTypeDefinition {
            code: "A320".parse()?,
            name: "Airbus A320-200".to_owned(),
            wake: WakeCategory::Medium,
            seats: 180,
            cruise_speed: Knots::new(450.0)?,
            cruise_altitude: Feet::new(37_000.0)?,
            climb_rate: FeetPerMinute::new(2500.0)?,
            descent_rate: FeetPerMinute::new(2000.0)?,
            approach_speed: Knots::new(136.0)?,
            range: NauticalMiles::new(3300.0)?,
            min_runway_length: Meters::new(2100.0)?,
            source: "test fixture".to_owned(),
        })
    }

    fn reason(definition: AircraftTypeDefinition) -> Result<&'static str> {
        let error = AircraftType::new(definition)
            .err()
            .context("must be rejected")?;
        match error
            .downcast::<CoreError>()
            .context("expected a CoreError")?
        {
            CoreError::InvalidAircraftType { reason, .. } => Ok(reason),
            other => anyhow::bail!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn test_aircraft_type_new_exposes_fields() -> Result<()> {
        let aircraft = AircraftType::new(a320()?)?;
        assert_eq!(aircraft.code().as_str(), "A320");
        assert_eq!(aircraft.name(), "Airbus A320-200");
        assert_eq!(aircraft.wake(), WakeCategory::Medium);
        assert_eq!(aircraft.seats(), 180);
        assert!((aircraft.cruise_speed().value() - 450.0).abs() < f64::EPSILON);
        assert!((aircraft.cruise_altitude().value() - 37_000.0).abs() < f64::EPSILON);
        assert!((aircraft.climb_rate().value() - 2500.0).abs() < f64::EPSILON);
        assert!((aircraft.descent_rate().value() - 2000.0).abs() < f64::EPSILON);
        assert!((aircraft.approach_speed().value() - 136.0).abs() < f64::EPSILON);
        assert!((aircraft.range().value() - 3300.0).abs() < f64::EPSILON);
        assert!((aircraft.min_runway_length().value() - 2100.0).abs() < f64::EPSILON);
        assert_eq!(aircraft.source(), "test fixture");
        Ok(())
    }

    #[test]
    fn test_aircraft_type_new_rejects_implausible_values() -> Result<()> {
        let mut no_seats = a320()?;
        no_seats.seats = 0;
        assert_eq!(reason(no_seats)?, "seats must be positive");

        let mut slow = a320()?;
        slow.cruise_speed = Knots::new(120.0)?;
        assert_eq!(
            reason(slow)?,
            "approach speed must be positive and below cruise speed"
        );

        let mut low = a320()?;
        low.cruise_altitude = Feet::new(500.0)?;
        assert_eq!(reason(low)?, "cruise altitude must be within 1000–51000 ft");

        let mut flat = a320()?;
        flat.climb_rate = FeetPerMinute::new(0.0)?;
        assert_eq!(reason(flat)?, "climb and descent rates must be positive");

        let mut short = a320()?;
        short.range = NauticalMiles::new(-1.0)?;
        assert_eq!(
            reason(short)?,
            "range and minimum runway length must be positive"
        );

        let mut unnamed = a320()?;
        unnamed.name = String::new();
        assert_eq!(reason(unnamed)?, "name and source must be non-empty");
        Ok(())
    }

    #[test]
    fn test_catalog_lookup_by_code() -> Result<()> {
        let catalog = AircraftCatalog::from_definition(AircraftCatalogDefinition {
            format_version: CATALOG_FORMAT_VERSION,
            types: vec![a320()?],
        })?;
        assert_eq!(catalog.len(), 1);
        assert!(!catalog.is_empty());
        let id = catalog.by_code("A320".parse()?).context("A320 present")?;
        assert_eq!(
            catalog.get(id).map(AircraftType::name),
            Some("Airbus A320-200")
        );
        assert!(catalog.by_code("B738".parse()?).is_none());
        assert_eq!(catalog.iter().count(), 1);
        Ok(())
    }

    #[test]
    fn test_catalog_rejects_duplicate_code() -> Result<()> {
        let error = AircraftCatalog::from_definition(AircraftCatalogDefinition {
            format_version: CATALOG_FORMAT_VERSION,
            types: vec![a320()?, a320()?],
        })
        .err()
        .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::DuplicateAircraftType {
                code: "A320".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn test_catalog_rejects_unknown_version() -> Result<()> {
        let error = AircraftCatalog::from_json(r#"{"format_version": 7, "types": []}"#)
            .err()
            .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "aircraft catalog",
                found: 7,
                supported: 1
            })
        );
        Ok(())
    }

    #[test]
    fn test_shipped_catalog_is_valid_and_complete() -> Result<()> {
        let catalog = AircraftCatalog::from_json(CATALOG).context("assets/aircraft/types.json")?;
        let expected = [
            "A319", "A320", "A20N", "A321", "A21N", "B738", "B38M", "BCS3", "E190", "E195", "CRJ9",
            "AT76", "DH8D", "A333", "A359", "B789", "B77W", "A388", "B748",
        ];
        assert_eq!(catalog.len(), expected.len());
        for code in expected {
            assert!(catalog.by_code(code.parse()?).is_some(), "{code} missing");
        }
        let a380 = catalog
            .by_code("A388".parse()?)
            .and_then(|id| catalog.get(id))
            .context("A388")?;
        assert_eq!(a380.wake(), WakeCategory::Super);
        Ok(())
    }

    #[test]
    fn test_catalog_json_round_trip() -> Result<()> {
        let catalog = AircraftCatalog::from_json(CATALOG)?;
        let again = AircraftCatalog::from_json(&catalog.to_definition().to_json_pretty()?)?;
        assert_eq!(again, catalog);
        Ok(())
    }
}
