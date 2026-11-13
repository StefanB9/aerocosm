//! The world file format.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{AirportDefinition, CountryDefinition, RunwayDefinition};
use crate::{
    CoreError,
    asset::{self, AssetKind},
};

/// World file format version this build reads and writes.
/// Version 2 added the optional airport `catchment`; version 3 the scheduled
/// flag, time zone and prevailing wind (plan M5); version 4 the countries
/// (plan M5 D109).
pub const FORMAT_VERSION: u32 = 4;
/// The oldest version this build still reads: version 2 files load with the
/// fields of later versions unset.
const OLDEST_VERSION: u32 = 2;

/// The world as a binary asset (plan M5 D104).
const WORLD_ASSET: AssetKind = AssetKind {
    tag: *b"WRLD",
    name: "world asset",
    version: FORMAT_VERSION,
};

/// The contents of a world file: unvalidated airports and runways.
///
/// Build a [`super::World`] from it to validate cross-references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDefinition {
    /// Must equal [`FORMAT_VERSION`].
    pub format_version: u32,
    /// All aerodromes.
    pub airports: Vec<AirportDefinition>,
    /// All runways, referencing airports by ident.
    pub runways: Vec<RunwayDefinition>,
    /// Countries with population and output (version 4); empty in older
    /// files.
    #[serde(default)]
    pub countries: Vec<CountryDefinition>,
}

impl WorldDefinition {
    /// Parses a world file, checking its format version.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition: Self = serde_json::from_str(text).context("malformed world file")?;
        if !(OLDEST_VERSION..=FORMAT_VERSION).contains(&definition.format_version) {
            return Err(CoreError::UnsupportedFileFormat {
                file: "world file",
                found: definition.format_version,
                supported: FORMAT_VERSION,
            }
            .into());
        }
        Ok(definition)
    }

    /// The world as a binary asset (plan M5 D104), in this build's version.
    pub fn to_asset(&self) -> Result<Vec<u8>> {
        let current = Self {
            format_version: FORMAT_VERSION,
            ..self.clone()
        };
        asset::encode(WORLD_ASSET, &current)
    }

    /// Reads a world asset written by [`Self::to_asset`].
    pub fn from_asset(bytes: &[u8]) -> Result<Self> {
        asset::decode(WORLD_ASSET, bytes)
    }

    /// Serialises as indented JSON with a trailing newline. Output is
    /// deterministic for a given definition, so regenerated files diff cleanly.
    pub fn to_json_pretty(&self) -> Result<String> {
        let mut json = serde_json::to_string_pretty(self).context("cannot serialise world")?;
        json.push('\n');
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        CoreError,
        world::{airport::tests::frankfurt, runway::tests::frankfurt_07c},
    };

    fn sample() -> Result<WorldDefinition> {
        Ok(WorldDefinition {
            format_version: FORMAT_VERSION,
            countries: Vec::new(),
            airports: vec![frankfurt()?],
            runways: vec![frankfurt_07c("EDDF")?],
        })
    }

    #[test]
    fn test_world_definition_json_round_trip() -> Result<()> {
        let definition = sample()?;
        let json = definition.to_json_pretty()?;
        assert!(json.starts_with("{\n  \"format_version\": 4,"), "{json}");
        assert!(json.ends_with('\n'), "files end with a newline");
        assert_eq!(WorldDefinition::from_json(&json)?, definition);
        Ok(())
    }

    #[test]
    fn test_world_definition_rejects_unknown_version() -> Result<()> {
        let json = sample()?
            .to_json_pretty()?
            .replace("\"format_version\": 4", "\"format_version\": 99");
        let error = WorldDefinition::from_json(&json)
            .err()
            .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "world file",
                found: 99,
                supported: FORMAT_VERSION
            })
        );
        Ok(())
    }

    #[test]
    fn test_world_definition_rejects_version_1_files() -> Result<()> {
        let json = sample()?
            .to_json_pretty()?
            .replace("\"format_version\": 4", "\"format_version\": 1");
        let error = WorldDefinition::from_json(&json)
            .err()
            .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "world file",
                found: 1,
                supported: FORMAT_VERSION
            })
        );
        Ok(())
    }

    /// The sample with the fields of version 3 set.
    fn global() -> Result<WorldDefinition> {
        let mut definition = sample()?;
        let airport = &mut definition.airports[0];
        airport.scheduled = true;
        airport.zone = Some(crate::time::Zone::new("Europe/Berlin")?);
        airport.wind = Some(crate::units::Degrees::new(240.0)?);
        Ok(definition)
    }

    #[test]
    fn test_world_definition_binary_round_trip() -> Result<()> {
        let definition = global()?;
        let bytes = definition.to_asset()?;
        assert_eq!(WorldDefinition::from_asset(&bytes)?, definition);
        // An airport without the optional values survives too.
        let plain = sample()?;
        assert_eq!(WorldDefinition::from_asset(&plain.to_asset()?)?, plain);
        Ok(())
    }

    #[test]
    fn test_world_definition_reads_version_2_files() -> Result<()> {
        // Version 2 had neither the scheduled flag nor zone and wind.
        let mut value: serde_json::Value =
            serde_json::from_str(&sample()?.to_json_pretty()?).context("json")?;
        value["format_version"] = 2.into();
        let airport = value["airports"][0].as_object_mut().context("an airport")?;
        for field in ["scheduled", "zone", "wind"] {
            airport.remove(field);
        }
        let definition = WorldDefinition::from_json(&value.to_string())?;
        let airport = &definition.airports[0];
        assert!(!airport.scheduled && airport.zone.is_none() && airport.wind.is_none());
        Ok(())
    }

    #[test]
    fn test_world_definition_rejects_invalid_nested_values() -> Result<()> {
        let json = sample()?.to_json_pretty()?.replace("\"EDDF\"", "\"eddf\"");
        let error = WorldDefinition::from_json(&json)
            .err()
            .context("must be rejected")?;
        assert!(
            format!("{error:#}").contains("invalid airport identifier"),
            "{error:#}"
        );
        Ok(())
    }

    #[test]
    fn test_world_definition_rejects_unknown_fields() -> Result<()> {
        let json = sample()?.to_json_pretty()?.replace(
            "\"format_version\": 4,",
            "\"format_version\": 4, \"extra\": 0,",
        );
        assert!(WorldDefinition::from_json(&json).is_err());
        Ok(())
    }
}
