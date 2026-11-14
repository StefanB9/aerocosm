//! The ground file format (plan M4 D65, D67): per airport, the taxi network
//! with stands, holding positions, runway exits and aprons.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::GroundName;
use crate::{
    CoreError,
    asset::{self, AssetKind},
    units::{Degrees, Meters},
    world::{RunwayDesignator, codes::AirportIdent},
};

/// Ground file format version this build reads and writes.
pub const FORMAT_VERSION: u32 = 2;

const GROUND_ASSET: AssetKind = AssetKind {
    tag: *b"GRND",
    name: "ground asset",
    version: FORMAT_VERSION,
};

/// The region of the ground asset an airport's layout lives in (plan M5
/// D117): the first letter of its ident (the ICAO region), or `None` for
/// local codes that start with a digit.
pub fn ground_region(airport: AirportIdent) -> Option<char> {
    airport
        .as_str()
        .chars()
        .next()
        .filter(char::is_ascii_uppercase)
}

/// The file name of a region's ground asset, e.g. `E.ground.bin`, or
/// `other.ground.bin` for `None`.
pub fn ground_file(region: Option<char>) -> String {
    region.map_or_else(
        || "other.ground.bin".to_owned(),
        |letter| format!("{letter}.ground.bin"),
    )
}

/// A position as `[latitude, longitude]` in degrees: compact for the
/// thousands of points of a hub's network.
pub type Point = [f64; 2];

/// What kind of pavement an edge runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A taxiway between runways and aprons.
    Taxiway,
    /// A taxi lane on an apron, leading to stands.
    Taxilane,
    /// Part of a runway's centreline.
    Runway,
    /// The line from a taxi lane into a stand.
    LeadIn,
}

/// One edge of the network, from node to node along a polyline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeDefinition {
    /// Index of the start node.
    pub from: u32,
    /// Index of the end node.
    pub to: u32,
    /// Pavement kind.
    pub kind: EdgeKind,
    /// Taxiway or runway name, e.g. "N7" or "07C/25C".
    #[serde(default)]
    pub name: Option<GroundName>,
    /// Only to be taxied from `from` to `to`.
    #[serde(default)]
    pub oneway: bool,
    /// Largest wingspan allowed, where published.
    #[serde(default)]
    pub max_wingspan: Option<Meters>,
    /// Points between the two nodes.
    #[serde(default)]
    pub points: Vec<Point>,
}

/// A parking stand: where an aircraft stops, nose in along `heading`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandDefinition {
    /// Stand designation, e.g. "A14" or "V155".
    pub name: GroundName,
    /// Index of the node the aircraft stops at.
    pub node: u32,
    /// True heading of a parked aircraft.
    pub heading: Degrees,
    /// Largest wingspan that fits.
    pub max_wingspan: Meters,
    /// Apron or terminal the stand belongs to, if known.
    #[serde(default)]
    pub area: Option<GroundName>,
    /// On an apron mapped for cargo (plan M5 D120).
    #[serde(default)]
    pub cargo: bool,
}

/// A runway holding position: departures from `runway` wait here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldingPositionDefinition {
    /// Index of the node on the taxiway.
    pub node: u32,
    /// The runway end departures line up at.
    pub runway: RunwayDesignator,
}

/// A runway exit: where a taxiway leaves a runway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitDefinition {
    /// The runway end landings use it from.
    pub runway: RunwayDesignator,
    /// Index of the node on the runway centreline.
    pub node: u32,
    /// Distance from the landing threshold.
    pub distance: Meters,
    /// Turn off the landing direction, 0–180°: up to ~45° for rapid exits.
    pub angle: Degrees,
}

/// The ground layout of one airport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirportGroundDefinition {
    /// The airport.
    pub airport: AirportIdent,
    /// Network nodes, referenced by index.
    pub nodes: Vec<Point>,
    /// Network edges.
    pub edges: Vec<EdgeDefinition>,
    /// Parking stands.
    pub stands: Vec<StandDefinition>,
    /// Runway holding positions.
    pub holding_positions: Vec<HoldingPositionDefinition>,
    /// Runway exits.
    pub exits: Vec<ExitDefinition>,
    /// Nodes with an intermediate holding position: where aircraft hold
    /// short of an intersection (plan M4 D89).
    #[serde(default)]
    pub intermediate_holds: Vec<u32>,
    /// Apron outlines, for drawing.
    #[serde(default)]
    pub aprons: Vec<Vec<Point>>,
}

/// The contents of a ground file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundDefinition {
    /// Must equal [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Attribution required by the data licence.
    pub attribution: String,
    /// Airports with a ground layout.
    pub airports: Vec<AirportGroundDefinition>,
}

impl GroundDefinition {
    /// Parses a ground file, checking its format version.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition: Self = serde_json::from_str(text).context("malformed ground file")?;
        if definition.format_version != FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "ground file",
                found: definition.format_version,
                supported: FORMAT_VERSION,
            }
            .into());
        }
        Ok(definition)
    }

    /// Serialises as JSON with one airport section per line group and a
    /// trailing newline; deterministic for a given definition.
    pub fn to_json(&self) -> Result<String> {
        let mut json = serde_json::to_string(self).context("cannot serialise ground")?;
        json.push('\n');
        Ok(json)
    }

    /// Encodes as a binary asset (plan M5 D104), in the current version.
    pub fn to_asset(&self) -> Result<Vec<u8>> {
        let current = Self {
            format_version: FORMAT_VERSION,
            ..self.clone()
        };
        asset::encode(GROUND_ASSET, &current)
    }

    /// Reads an asset written by [`Self::to_asset`].
    pub fn from_asset(bytes: &[u8]) -> Result<Self> {
        asset::decode(GROUND_ASSET, bytes)
    }

    /// The layouts of all `parts` (regions) in one definition; an airport
    /// mapped twice is an error.
    pub fn combine(parts: impl IntoIterator<Item = Self>) -> Result<Self> {
        let mut combined = Self {
            format_version: FORMAT_VERSION,
            attribution: String::new(),
            airports: Vec::new(),
        };
        let mut seen = std::collections::BTreeSet::new();
        for part in parts {
            if combined.attribution.is_empty() {
                combined.attribution = part.attribution;
            }
            for airport in part.airports {
                if !seen.insert(airport.airport) {
                    return Err(CoreError::InvalidGround {
                        airport: airport.airport.to_string(),
                        reason: "mapped in two ground assets",
                    }
                    .into());
                }
                combined.airports.push(airport);
            }
        }
        Ok(combined)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use anyhow::{Context, Result};

    use super::*;

    /// A tiny airport: a stand, a lead-in, a taxiway to a holding position
    /// and a runway exit.
    pub(crate) fn sample() -> Result<AirportGroundDefinition> {
        Ok(AirportGroundDefinition {
            airport: "EDDF".parse()?,
            nodes: vec![[50.030, 8.560], [50.031, 8.560], [50.033, 8.560]],
            edges: vec![
                EdgeDefinition {
                    from: 1,
                    to: 0,
                    kind: EdgeKind::LeadIn,
                    name: None,
                    oneway: false,
                    max_wingspan: None,
                    points: Vec::new(),
                },
                EdgeDefinition {
                    from: 1,
                    to: 2,
                    kind: EdgeKind::Taxiway,
                    name: Some("N7".parse()?),
                    oneway: false,
                    max_wingspan: Some(Meters::new(65.0)?),
                    points: vec![[50.032, 8.5601]],
                },
            ],
            stands: vec![StandDefinition {
                name: "A14".parse()?,
                node: 0,
                heading: Degrees::new(180.0)?,
                max_wingspan: Meters::new(36.0)?,
                area: Some("T1".parse()?),
                cargo: false,
            }],
            holding_positions: vec![HoldingPositionDefinition {
                node: 2,
                runway: "25C".parse()?,
            }],
            exits: vec![ExitDefinition {
                runway: "07C".parse()?,
                node: 2,
                distance: Meters::new(2_400.0)?,
                angle: Degrees::new(30.0)?,
            }],
            intermediate_holds: vec![1],
            aprons: vec![vec![[50.029, 8.559], [50.029, 8.561], [50.0305, 8.561]]],
        })
    }

    fn file() -> Result<GroundDefinition> {
        Ok(GroundDefinition {
            format_version: FORMAT_VERSION,
            attribution: "© OpenStreetMap contributors".to_owned(),
            airports: vec![sample()?],
        })
    }

    #[test]
    fn test_ground_definition_round_trips() -> Result<()> {
        let definition = file()?;
        let json = definition.to_json()?;
        assert!(json.ends_with('\n'));
        assert!(json.starts_with("{\"format_version\":2,"), "{json}");
        assert_eq!(GroundDefinition::from_json(&json)?, definition);
        Ok(())
    }

    #[test]
    fn test_ground_asset_round_trips() -> Result<()> {
        let definition = file()?;
        let bytes = definition.to_asset()?;
        assert!(bytes.starts_with(b"AEROGRND"));
        assert_eq!(GroundDefinition::from_asset(&bytes)?, definition);
        let error = GroundDefinition::from_asset(b"AEROWRLD\x02\0\0\0")
            .err()
            .context("a world asset is not ground")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::NotAnAsset {
                file: "ground asset"
            })
        );
        Ok(())
    }

    #[test]
    fn test_ground_regions_follow_the_ident() -> Result<()> {
        assert_eq!(ground_region("EDDF".parse()?), Some('E'));
        assert_eq!(ground_region("KLAX".parse()?), Some('K'));
        assert_eq!(ground_region("03N".parse()?), None);
        assert_eq!(ground_file(Some('E')), "E.ground.bin");
        assert_eq!(ground_file(None), "other.ground.bin");
        Ok(())
    }

    #[test]
    fn test_ground_definitions_combine_without_duplicates() -> Result<()> {
        let mut other = file()?;
        other.airports[0].airport = "EDDM".parse()?;
        let combined = GroundDefinition::combine([file()?, other.clone()])?;
        let idents: Vec<String> = combined
            .airports
            .iter()
            .map(|a| a.airport.to_string())
            .collect();
        assert_eq!(idents, ["EDDF", "EDDM"]);
        assert_eq!(combined.attribution, "© OpenStreetMap contributors");
        assert!(GroundDefinition::combine([file()?, file()?]).is_err());
        assert_eq!(GroundDefinition::combine([])?.airports, []);
        Ok(())
    }

    #[test]
    fn test_ground_definition_rejects_other_versions() -> Result<()> {
        let json = file()?
            .to_json()?
            .replace("\"format_version\":2", "\"format_version\":7");
        let error = GroundDefinition::from_json(&json)
            .err()
            .context("version 7 must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "ground file",
                found: 7,
                supported: FORMAT_VERSION,
            })
        );
        Ok(())
    }

    #[test]
    fn test_ground_definition_rejects_unknown_fields() -> Result<()> {
        let json = file()?
            .to_json()?
            .replace("\"kind\":\"lead_in\"", "\"kind\":\"lead_in\",\"speed\":3");
        assert!(GroundDefinition::from_json(&json).is_err());
        Ok(())
    }
}
