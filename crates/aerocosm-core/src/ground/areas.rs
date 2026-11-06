//! Curated stand areas (plan M4 D68): which stands an airline prefers at a
//! hub, by stand-name prefix, and the choice of a stand for an aircraft.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{AirportGround, GroundName, Stand};
use crate::{
    CoreError, Id,
    units::Meters,
    world::{
        Airport, World,
        codes::{AirlineIcao, AirportIdent},
    },
};

/// Stand areas file format version this build reads and writes.
pub const AREAS_FORMAT_VERSION: u32 = 1;

/// Unused wingspan of a stand is compared in steps of this many metres, so
/// small aircraft do not take large stands while small ones are free.
const WASTE_STEP: f64 = 15.0;

/// One area: stands whose names start with one of `stands`, preferred by
/// `airlines`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDefinition {
    /// Name, e.g. "Terminal 1".
    pub name: String,
    /// Stand-name prefixes, e.g. `["A", "B"]`.
    pub stands: Vec<GroundName>,
    /// Airlines that use the area.
    pub airlines: Vec<AirlineIcao>,
    /// Only the listed airlines use the area (e.g. cargo aprons).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclusive: bool,
}

/// The areas of one airport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirportAreasDefinition {
    /// The airport.
    pub airport: AirportIdent,
    /// Its areas.
    pub areas: Vec<AreaDefinition>,
}

/// The contents of the stand areas file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandAreasDefinition {
    /// Must equal [`AREAS_FORMAT_VERSION`].
    pub format_version: u32,
    /// Where the assignments come from.
    pub source: String,
    /// Airports with curated areas.
    pub airports: Vec<AirportAreasDefinition>,
}

impl StandAreasDefinition {
    /// Parses a stand areas file, checking its format version.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition: Self = serde_json::from_str(text).context("malformed stand areas file")?;
        if definition.format_version != AREAS_FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "stand areas file",
                found: definition.format_version,
                supported: AREAS_FORMAT_VERSION,
            }
            .into());
        }
        Ok(definition)
    }
}

/// Curated stand areas by airport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StandAreas {
    by_airport: BTreeMap<Id<Airport>, Vec<AreaDefinition>>,
}

impl StandAreas {
    /// No curated areas.
    pub const fn empty() -> Self {
        Self {
            by_airport: BTreeMap::new(),
        }
    }

    /// Resolves the airports of `definition` in `world`.
    pub fn new(definition: StandAreasDefinition, world: &World) -> Result<Self> {
        let mut by_airport = BTreeMap::new();
        for airport in definition.airports {
            let id = world.airport_by_ident(airport.airport).ok_or_else(|| {
                CoreError::UnknownAirport {
                    ident: airport.airport.to_string(),
                }
            })?;
            by_airport.insert(id, airport.areas);
        }
        Ok(Self { by_airport })
    }

    /// The areas at `airport`.
    pub fn areas(&self, airport: Id<Airport>) -> &[AreaDefinition] {
        self.by_airport.get(&airport).map_or(&[], Vec::as_slice)
    }
}

fn matches(stand: &Stand, prefixes: &[GroundName]) -> bool {
    prefixes
        .iter()
        .any(|prefix| stand.name().as_str().starts_with(prefix.as_str()))
}

/// What a stand is wanted for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandRequest {
    /// The operating airline.
    pub airline: AirlineIcao,
    /// The aircraft's wingspan.
    pub wingspan: Meters,
    /// Where the aircraft comes from (e.g. its runway exit); nearer stands
    /// are preferred.
    pub near: crate::LatLon,
}

impl AirportGround {
    /// The best stand for `request` among those `free` reports free, with
    /// their variants: the airline's own area first, then the stand whose
    /// size fits best, then the nearest. Stands in other airlines' exclusive
    /// areas are never chosen. `None` if nothing fits.
    pub fn choose_stand(
        &self,
        areas: &[AreaDefinition],
        request: &StandRequest,
        free: impl Fn(Id<Stand>) -> bool,
    ) -> Option<Id<Stand>> {
        let own: Vec<&AreaDefinition> = areas
            .iter()
            .filter(|area| area.airlines.contains(&request.airline))
            .collect();
        let barred: Vec<&AreaDefinition> = areas
            .iter()
            .filter(|area| area.exclusive && !area.airlines.contains(&request.airline))
            .collect();
        let mut best: Option<((u8, u32, f64), Id<Stand>)> = None;
        for (index, stand) in self.stands().iter().enumerate() {
            let Ok(id) = Id::<Stand>::from_usize(index) else {
                continue;
            };
            if stand.max_wingspan() < request.wingspan
                || barred.iter().any(|area| matches(stand, &area.stands))
                || !free(id)
                || !self.variants(id).iter().all(|&variant| free(variant))
            {
                continue;
            }
            let preferred = own.iter().any(|area| matches(stand, &area.stands));
            // SAFETY: a small non-negative number of steps.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let waste = ((stand.max_wingspan() - request.wingspan).value() / WASTE_STEP) as u32;
            let distance = self.nodes()[stand.node().as_usize()]
                .position()
                .distance_to(request.near)
                .value();
            let key = (u8::from(!preferred), waste, distance);
            if best.is_none_or(|(least, _)| key < least) {
                best = Some((key, id));
            }
        }
        best.map(|(_, id)| id)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        LatLon,
        ground::{GroundNetwork, definition::tests::sample},
        test_data::data,
        units::Degrees,
    };

    fn ground(stands: &[(&str, [f64; 2], f64)]) -> Result<AirportGround> {
        let mut definition = sample()?;
        let template = definition.stands[0].clone();
        definition.stands.clear();
        for (name, point, span) in stands {
            definition.nodes.push(*point);
            let mut stand = template.clone();
            stand.name = name.parse()?;
            stand.node = u32::try_from(definition.nodes.len() - 1)?;
            stand.max_wingspan = Meters::new(*span)?;
            stand.heading = Degrees::new(0.0)?;
            definition.stands.push(stand);
        }
        AirportGround::new(definition, &data()?.world)
    }

    fn area(
        name: &str,
        stands: &[&str],
        airlines: &[&str],
        exclusive: bool,
    ) -> Result<AreaDefinition> {
        Ok(AreaDefinition {
            name: name.to_owned(),
            stands: stands.iter().map(|s| s.parse()).collect::<Result<_>>()?,
            airlines: airlines.iter().map(|s| s.parse()).collect::<Result<_>>()?,
            exclusive,
        })
    }

    fn request(airline: &str, span: f64) -> Result<StandRequest> {
        Ok(StandRequest {
            airline: airline.parse()?,
            wingspan: Meters::new(span)?,
            near: LatLon::new(50.03, 8.56)?,
        })
    }

    fn name(ground: &AirportGround, stand: Option<Id<Stand>>) -> Option<String> {
        stand.and_then(|id| {
            ground
                .stands()
                .get(id.as_usize())
                .map(|s| s.name().to_string())
        })
    }

    #[test]
    fn test_choose_stand_prefers_own_area_then_fit_then_distance() -> Result<()> {
        let ground = ground(&[
            ("V1", [50.0301, 8.5601], 40.0),
            ("A1", [50.0350, 8.5700], 80.0),
            ("A2", [50.0360, 8.5700], 40.0),
            ("A3", [50.0370, 8.5700], 40.0),
            ("F1", [50.0300, 8.5600], 80.0),
        ])?;
        let areas = [
            area("Terminal 1", &["A"], &["DLH"], false)?,
            area("Cargo", &["F"], &["BOX"], true)?,
        ];
        let all = |_| true;
        // Lufthansa A320: its terminal, the tight stand, the nearer one.
        let dlh = ground.choose_stand(&areas, &request("DLH", 35.8)?, all);
        assert_eq!(name(&ground, dlh), Some("A2".to_owned()));
        // A widebody needs A1.
        let wide = ground.choose_stand(&areas, &request("DLH", 64.8)?, all);
        assert_eq!(name(&ground, wide), Some("A1".to_owned()));
        // Air France has no area here: the nearest fitting stand that is not
        // reserved for cargo.
        let afr = ground.choose_stand(&areas, &request("AFR", 35.8)?, all);
        assert_eq!(name(&ground, afr), Some("V1".to_owned()));
        let cargo = ground.choose_stand(&areas, &request("BOX", 64.8)?, all);
        assert_eq!(name(&ground, cargo), Some("F1".to_owned()));
        // Nothing fits an A380 at 80 m except A1 and F1; with A1 taken,
        // Air France gets nothing.
        let taken = |id: Id<Stand>| id.index() != 1;
        assert_eq!(
            ground.choose_stand(&areas, &request("AFR", 79.8)?, taken),
            None
        );
        // With A2 taken, Lufthansa falls back to A3.
        let a2_taken = |id: Id<Stand>| id.index() != 2;
        let next = ground.choose_stand(&areas, &request("DLH", 35.8)?, a2_taken);
        assert_eq!(name(&ground, next), Some("A3".to_owned()));
        Ok(())
    }

    #[test]
    fn test_choose_stand_skips_stands_whose_variant_is_taken() -> Result<()> {
        let ground = ground(&[
            ("05", [50.0300, 8.5600], 60.0),
            ("05A", [50.0300, 8.5602], 40.0),
            ("06", [50.0400, 8.5800], 40.0),
        ])?;
        assert_eq!(ground.variants(Id::new(1)), &[Id::new(0)]);
        let wide_parked = |id: Id<Stand>| id.index() != 0;
        let chosen = ground.choose_stand(&[], &request("DLH", 35.8)?, wide_parked);
        assert_eq!(name(&ground, chosen), Some("06".to_owned()));
        Ok(())
    }

    #[test]
    fn test_stand_areas_file_loads_and_resolves_airports() -> Result<()> {
        let world = data()?.world;
        let text = include_str!("../../../../assets/ground/stand-areas.json");
        let areas = StandAreas::new(StandAreasDefinition::from_json(text)?, &world)?;
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        assert!(areas.areas(frankfurt).len() >= 2);
        let hamburg = world.airport_by_ident("EDDH".parse()?).context("EDDH")?;
        assert_eq!(areas.areas(hamburg).len(), 0);
        // Every prefix matches stands in the shipped layouts.
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        for (airport, list) in &areas.by_airport {
            let layout = network.airport(*airport).context("a layout")?;
            for area in list {
                for prefix in &area.stands {
                    let found = layout
                        .stands()
                        .iter()
                        .any(|s| s.name().as_str().starts_with(prefix.as_str()));
                    assert!(found, "{} {prefix}", area.name);
                }
            }
        }
        let bad = text.replace("\"format_version\": 1", "\"format_version\": 9");
        assert!(StandAreasDefinition::from_json(&bad).is_err());
        let unknown = text.replace("\"EDDM\"", "\"EDXX\"");
        assert!(StandAreas::new(StandAreasDefinition::from_json(&unknown)?, &world).is_err());
        Ok(())
    }
}
