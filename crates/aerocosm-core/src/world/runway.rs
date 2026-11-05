//! Runways and their geometry.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{Airport, RunwayDesignator, codes::AirportIdent};
use crate::{
    CoreError, Id, LatLon,
    units::{Degrees, Meters},
};

/// Largest accepted deviation from exactly opposite end headings.
const MAX_HEADING_DEVIATION: f64 = 5.0;
/// Accepted ratio of threshold-to-threshold distance to declared length.
/// Displaced thresholds make it shorter; beyond these bounds the data is
/// inconsistent.
const THRESHOLD_DISTANCE_RATIO: std::ops::RangeInclusive<f64> = 0.5..=1.2;

/// Runway surface, normalised from free-text source data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Asphalt, concrete or similar.
    Paved,
    /// Grass.
    Grass,
    /// Gravel, dirt, sand or similar.
    Unpaved,
    /// A water runway.
    Water,
    /// Not known.
    Unknown,
}

/// Where a runway's geometry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometrySource {
    /// Threshold coordinates from the source data.
    Surveyed,
    /// Synthesised from the reference point, heading and length; to be replaced
    /// by better data (e.g. `OpenStreetMap`) later.
    Approximate,
}

/// One end of a runway.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawRunwayEnd")]
pub struct RunwayEnd {
    designator: RunwayDesignator,
    threshold: LatLon,
    true_heading: Degrees,
    displaced_threshold: Meters,
}

/// Unvalidated wire form of [`RunwayEnd`].
#[derive(Deserialize)]
struct RawRunwayEnd {
    designator: RunwayDesignator,
    threshold: LatLon,
    true_heading: Degrees,
    displaced_threshold: Meters,
}

impl TryFrom<RawRunwayEnd> for RunwayEnd {
    type Error = anyhow::Error;

    fn try_from(raw: RawRunwayEnd) -> Result<Self> {
        Self::new(
            raw.designator,
            raw.threshold,
            raw.true_heading,
            raw.displaced_threshold,
        )
    }
}

impl RunwayEnd {
    /// Builds a runway end; the heading is normalised to `[0, 360)`.
    pub fn new(
        designator: RunwayDesignator,
        threshold: LatLon,
        true_heading: Degrees,
        displaced_threshold: Meters,
    ) -> Result<Self> {
        if displaced_threshold.value() < 0.0 {
            return Err(CoreError::InvalidRunwayEnd {
                designator: designator.to_string(),
                reason: "displaced threshold must not be negative",
            }
            .into());
        }
        Ok(Self {
            designator,
            threshold,
            true_heading: true_heading.normalized(),
            displaced_threshold,
        })
    }

    /// The end's designator.
    pub const fn designator(&self) -> RunwayDesignator {
        self.designator
    }

    /// Position of the runway end.
    pub const fn threshold(&self) -> LatLon {
        self.threshold
    }

    /// True heading when departing from or landing towards this end.
    pub const fn true_heading(&self) -> Degrees {
        self.true_heading
    }

    /// Distance from the runway end to the landing threshold.
    pub const fn displaced_threshold(&self) -> Meters {
        self.displaced_threshold
    }
}

/// Unvalidated runway data as stored in the world file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunwayDefinition {
    /// The airport this runway belongs to.
    pub airport: AirportIdent,
    /// The two ends, lower designator first by convention.
    pub ends: [RunwayEnd; 2],
    /// Declared length.
    pub length: Meters,
    /// Declared width.
    pub width: Meters,
    /// Surface type.
    pub surface: Surface,
    /// Provenance of the end coordinates.
    pub geometry: GeometrySource,
}

/// A validated runway.
#[derive(Debug, Clone, PartialEq)]
pub struct Runway {
    airport: Id<Airport>,
    ends: [RunwayEnd; 2],
    length: Meters,
    width: Meters,
    surface: Surface,
    geometry: GeometrySource,
}

impl Runway {
    /// Validates a definition belonging to `airport`: positive dimensions,
    /// reciprocal designators and headings, and threshold positions consistent
    /// with the length.
    pub fn new(airport: Id<Airport>, definition: &RunwayDefinition) -> Result<Self> {
        let [first, second] = definition.ends;
        let invalid = |reason| CoreError::InvalidRunway {
            airport: definition.airport.to_string(),
            designator: format!("{}/{}", first.designator, second.designator),
            reason,
        };
        if definition.length.value() <= 0.0 || definition.width.value() <= 0.0 {
            return Err(invalid("length and width must be positive").into());
        }
        if first.designator.reciprocal() != second.designator {
            return Err(invalid("designators must be reciprocal").into());
        }
        let opposite = (second.true_heading.value() - first.true_heading.value()).rem_euclid(360.0);
        if (opposite - 180.0).abs() > MAX_HEADING_DEVIATION {
            return Err(invalid("end headings must be reciprocal within 5°").into());
        }
        let ratio =
            first.threshold.distance_to(second.threshold).value() / definition.length.value();
        if !THRESHOLD_DISTANCE_RATIO.contains(&ratio) {
            return Err(invalid("threshold distance must be 0.5–1.2 × length").into());
        }
        Ok(Self {
            airport,
            ends: definition.ends,
            length: definition.length,
            width: definition.width,
            surface: definition.surface,
            geometry: definition.geometry,
        })
    }

    /// The airport this runway belongs to.
    pub const fn airport(&self) -> Id<Airport> {
        self.airport
    }

    /// Both ends.
    pub const fn ends(&self) -> &[RunwayEnd; 2] {
        &self.ends
    }

    /// Declared length.
    pub const fn length(&self) -> Meters {
        self.length
    }

    /// Declared width.
    pub const fn width(&self) -> Meters {
        self.width
    }

    /// Surface type.
    pub const fn surface(&self) -> Surface {
        self.surface
    }

    /// Provenance of the end coordinates.
    pub const fn geometry(&self) -> GeometrySource {
        self.geometry
    }

    /// Display name such as `07C/25C`.
    pub fn name(&self) -> String {
        format!("{}/{}", self.ends[0].designator, self.ends[1].designator)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    /// Frankfurt 07C/25C: 4000 m, surveyed.
    pub(in crate::world) fn frankfurt_07c(airport: &str) -> Result<RunwayDefinition> {
        let west = LatLon::new(50.032_6, 8.534_6)?;
        let heading = Degrees::new(69.6)?;
        let east = west.destination(heading, Meters::new(4000.0)?);
        Ok(RunwayDefinition {
            airport: airport.parse()?,
            ends: [
                RunwayEnd::new("07C".parse()?, west, heading, Meters::new(0.0)?)?,
                RunwayEnd::new(
                    "25C".parse()?,
                    east,
                    Degrees::new(249.6)?,
                    Meters::new(0.0)?,
                )?,
            ],
            length: Meters::new(4000.0)?,
            width: Meters::new(60.0)?,
            surface: Surface::Paved,
            geometry: GeometrySource::Surveyed,
        })
    }

    fn rejection(definition: &RunwayDefinition) -> Result<CoreError> {
        let error = Runway::new(Id::new(0), definition)
            .err()
            .context("must be rejected")?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    fn invalid(reason: &'static str) -> CoreError {
        CoreError::InvalidRunway {
            airport: "EDDF".to_owned(),
            designator: "07C/25C".to_owned(),
            reason,
        }
    }

    #[test]
    fn test_runway_end_new_normalizes_heading() -> Result<()> {
        let end = RunwayEnd::new(
            "36".parse()?,
            LatLon::new(50.0, 8.0)?,
            Degrees::new(360.0)?,
            Meters::new(0.0)?,
        )?;
        assert!(end.true_heading().value().abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn test_runway_end_rejects_negative_displacement() -> Result<()> {
        let result = RunwayEnd::new(
            "36".parse()?,
            LatLon::new(50.0, 8.0)?,
            Degrees::new(0.0)?,
            Meters::new(-1.0)?,
        );
        let error = result.err().context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::InvalidRunwayEnd {
                designator: "36".to_owned(),
                reason: "displaced threshold must not be negative"
            })
        );
        Ok(())
    }

    #[test]
    fn test_runway_new_accepts_consistent_definition() -> Result<()> {
        let runway = Runway::new(Id::new(3), &frankfurt_07c("EDDF")?)?;
        assert_eq!(runway.airport(), Id::new(3));
        assert_eq!(runway.ends()[0].designator().to_string(), "07C");
        assert!((runway.length().value() - 4000.0).abs() < f64::EPSILON);
        assert!((runway.width().value() - 60.0).abs() < f64::EPSILON);
        assert_eq!(runway.surface(), Surface::Paved);
        assert_eq!(runway.geometry(), GeometrySource::Surveyed);
        assert_eq!(runway.name(), "07C/25C");
        Ok(())
    }

    #[test]
    fn test_runway_new_rejects_non_positive_dimensions() -> Result<()> {
        let mut definition = frankfurt_07c("EDDF")?;
        definition.width = Meters::new(0.0)?;
        assert_eq!(
            rejection(&definition)?,
            invalid("length and width must be positive")
        );
        Ok(())
    }

    #[test]
    fn test_runway_new_rejects_non_reciprocal_designators() -> Result<()> {
        let mut definition = frankfurt_07c("EDDF")?;
        let [first, second] = definition.ends;
        definition.ends = [
            first,
            RunwayEnd::new(
                "25L".parse()?,
                second.threshold(),
                second.true_heading(),
                Meters::new(0.0)?,
            )?,
        ];
        let error = rejection(&definition)?;
        assert_eq!(
            error,
            CoreError::InvalidRunway {
                airport: "EDDF".to_owned(),
                designator: "07C/25L".to_owned(),
                reason: "designators must be reciprocal"
            }
        );
        Ok(())
    }

    #[test]
    fn test_runway_new_rejects_non_reciprocal_headings() -> Result<()> {
        let mut definition = frankfurt_07c("EDDF")?;
        let [first, second] = definition.ends;
        definition.ends = [
            first,
            RunwayEnd::new(
                second.designator(),
                second.threshold(),
                Degrees::new(240.0)?,
                Meters::new(0.0)?,
            )?,
        ];
        assert_eq!(
            rejection(&definition)?,
            invalid("end headings must be reciprocal within 5°")
        );
        Ok(())
    }

    #[test]
    fn test_runway_new_rejects_thresholds_inconsistent_with_length() -> Result<()> {
        for length in [2000.0, 9000.0] {
            let mut definition = frankfurt_07c("EDDF")?;
            definition.length = Meters::new(length)?;
            assert_eq!(
                rejection(&definition)?,
                invalid("threshold distance must be 0.5–1.2 × length")
            );
        }
        Ok(())
    }

    #[test]
    fn test_runway_definition_serde_round_trip() -> Result<()> {
        let definition = frankfurt_07c("EDDF")?;
        let json = serde_json::to_string(&definition).context("serialise")?;
        assert!(json.contains(r#""surface":"paved""#), "{json}");
        assert!(json.contains(r#""geometry":"surveyed""#), "{json}");
        let back: RunwayDefinition = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back, definition);
        Ok(())
    }
}
