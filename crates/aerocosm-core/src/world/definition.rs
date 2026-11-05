//! The world file format.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{AirportDefinition, RunwayDefinition};
use crate::CoreError;

/// World file format version this build reads and writes.
/// Version 2 added the optional airport `catchment`.
pub const FORMAT_VERSION: u32 = 2;

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
}

impl WorldDefinition {
    /// Parses a world file, checking its format version.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition: Self = serde_json::from_str(text).context("malformed world file")?;
        if definition.format_version != FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "world file",
                found: definition.format_version,
                supported: FORMAT_VERSION,
            }
            .into());
        }
        Ok(definition)
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
            airports: vec![frankfurt()?],
            runways: vec![frankfurt_07c("EDDF")?],
        })
    }

    #[test]
    fn test_world_definition_json_round_trip() -> Result<()> {
        let definition = sample()?;
        let json = definition.to_json_pretty()?;
        assert!(json.starts_with("{\n  \"format_version\": 2,"), "{json}");
        assert!(json.ends_with('\n'), "files end with a newline");
        assert_eq!(WorldDefinition::from_json(&json)?, definition);
        Ok(())
    }

    #[test]
    fn test_world_definition_rejects_unknown_version() -> Result<()> {
        let json = sample()?
            .to_json_pretty()?
            .replace("\"format_version\": 2", "\"format_version\": 99");
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
            .replace("\"format_version\": 2", "\"format_version\": 1");
        let error = WorldDefinition::from_json(&json)
            .err()
            .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "world file",
                found: 1,
                supported: 2
            })
        );
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
            "\"format_version\": 2,",
            "\"format_version\": 2, \"extra\": 0,",
        );
        assert!(WorldDefinition::from_json(&json).is_err());
        Ok(())
    }
}
