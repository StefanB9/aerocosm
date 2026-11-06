//! Traffic calibration targets: what the generated schedules must reproduce
//! (plan D28).

mod clock;
mod operations;

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub(crate) use self::operations::angle_between;
pub use self::{
    clock::{ClockTime, Curfew},
    operations::{Capacity, RunwayMode, mode_for_wind},
};
use crate::{
    CoreError, LatLon,
    time::{Weekday, Zone},
    world::{World, codes::AirportIdent},
};

/// Traffic targets file format version this build reads and writes.
pub const TRAFFIC_FORMAT_VERSION: u32 = 2;

/// Days per year used to turn annual statistics into daily targets.
const DAYS_PER_YEAR: f64 = 365.0;
/// Weekday factors must average 1 within this tolerance.
const WEEKDAY_MEAN_TOLERANCE: f64 = 0.02;

/// Calibration for one commercial airport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirportTarget {
    /// The airport.
    pub airport: AirportIdent,
    /// Scheduled and charter movements (arrivals + departures) per year.
    pub movements_per_year: u32,
    /// Local time zone.
    pub zone: Zone,
    /// Night window without scheduled movements, in local time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curfew: Option<Curfew>,
    /// Local centre times of hub connecting banks; empty for non-hubs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub banks: Vec<ClockTime>,
    /// Fraction of the movements flown by cargo carriers (estimate, plan D41);
    /// absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_share: Option<f64>,
    /// Declared capacity of a coordinated airport (plan D54); absent means
    /// not coordinated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Capacity>,
    /// Runway operating modes by flow (plan D55); empty means the
    /// best-aligned runway in mixed mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runway_modes: Vec<RunwayMode>,
}

impl AirportTarget {
    /// The airport.
    pub const fn airport(&self) -> AirportIdent {
        self.airport
    }

    /// Scheduled and charter movements per year.
    pub const fn movements_per_year(&self) -> u32 {
        self.movements_per_year
    }

    /// Target movements on `weekday`, given the network's weekday factors.
    pub fn movements_per_day(&self, weekday: Weekday, factors: &[f64; 7]) -> f64 {
        f64::from(self.movements_per_year) / DAYS_PER_YEAR * factors[weekday.index()]
    }

    /// Local time zone.
    pub const fn zone(&self) -> &Zone {
        &self.zone
    }

    /// Night curfew, if any.
    pub const fn curfew(&self) -> Option<Curfew> {
        self.curfew
    }

    /// Hub bank times, earliest first.
    pub fn banks(&self) -> &[ClockTime] {
        &self.banks
    }

    /// Fraction of the movements flown by cargo carriers, `0..=1`.
    pub fn cargo_share(&self) -> f64 {
        self.cargo_share.unwrap_or(0.0)
    }

    /// Declared capacity, if the airport is coordinated.
    pub const fn capacity(&self) -> Option<Capacity> {
        self.capacity
    }
}

/// The time zone of an edge airport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeZone {
    /// The airport.
    pub airport: AirportIdent,
    /// Its local time zone.
    pub zone: Zone,
    /// Runway operating modes by flow; empty means the best-aligned runway.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runway_modes: Vec<RunwayMode>,
}

/// The contents of a traffic targets file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficDefinition {
    /// Must equal [`TRAFFIC_FORMAT_VERSION`].
    pub format_version: u32,
    /// Where the figures come from.
    pub source: String,
    /// Relative traffic per weekday, Monday first, averaging 1.
    pub weekday_factors: [f64; 7],
    /// Flights per day crossing the airspace without landing in it.
    pub overflights_per_day: f64,
    /// Coarse outline of the simulated airspace, a closed ring.
    pub airspace: Vec<LatLon>,
    /// Commercial airports with movement targets.
    pub airports: Vec<AirportTarget>,
    /// Time zones of the edge airports.
    pub edges: Vec<EdgeZone>,
}

impl TrafficDefinition {
    /// Serialises as indented JSON with a trailing newline.
    pub fn to_json_pretty(&self) -> Result<String> {
        let mut json =
            serde_json::to_string_pretty(self).context("cannot serialise traffic targets")?;
        json.push('\n');
        Ok(json)
    }
}

/// Validated traffic targets with lookups.
#[derive(Debug, Clone, PartialEq)]
pub struct TrafficTargets {
    definition: TrafficDefinition,
    by_airport: HashMap<AirportIdent, usize>,
    zones: HashMap<AirportIdent, Zone>,
    modes: HashMap<AirportIdent, Vec<RunwayMode>>,
}

impl TrafficTargets {
    /// Validates weekday factors, the airspace ring, airport targets and banks.
    pub fn from_definition(definition: TrafficDefinition) -> Result<Self> {
        if definition.format_version != TRAFFIC_FORMAT_VERSION {
            return Err(CoreError::UnsupportedFileFormat {
                file: "traffic targets",
                found: definition.format_version,
                supported: TRAFFIC_FORMAT_VERSION,
            }
            .into());
        }
        let invalid = |reason| CoreError::InvalidTraffic { reason };
        let factors = &definition.weekday_factors;
        let mean = factors.iter().sum::<f64>() / 7.0;
        if !factors.iter().all(|f| f.is_finite() && *f > 0.0)
            || (mean - 1.0).abs() > WEEKDAY_MEAN_TOLERANCE
        {
            return Err(invalid("weekday factors must be positive and average 1").into());
        }
        if !definition.overflights_per_day.is_finite() || definition.overflights_per_day < 0.0 {
            return Err(invalid("overflights per day must not be negative").into());
        }
        let ring = &definition.airspace;
        if ring.len() < 4 || ring.first() != ring.last() {
            return Err(invalid("airspace must be a closed ring of at least 4 points").into());
        }
        let mut by_airport = HashMap::with_capacity(definition.airports.len());
        let mut zones = HashMap::with_capacity(definition.airports.len() + definition.edges.len());
        for (index, target) in definition.airports.iter().enumerate() {
            if target.movements_per_year == 0 {
                return Err(invalid("airport targets must be positive").into());
            }
            if !target
                .cargo_share
                .is_none_or(|share| (0.0..=1.0).contains(&share))
            {
                return Err(invalid("cargo share must be within 0..=1").into());
            }
            if let Some(capacity) = &target.capacity {
                capacity.validate()?;
            }
            for mode in &target.runway_modes {
                mode.validate()?;
            }
            if !target.banks.windows(2).all(|w| w[0] < w[1]) {
                return Err(invalid("banks must be strictly increasing").into());
            }
            if by_airport.insert(target.airport, index).is_some()
                || zones.insert(target.airport, target.zone.clone()).is_some()
            {
                return Err(invalid("each airport may appear only once").into());
            }
        }
        for edge in &definition.edges {
            for mode in &edge.runway_modes {
                mode.validate()?;
            }
            if zones.insert(edge.airport, edge.zone.clone()).is_some() {
                return Err(invalid("each airport may appear only once").into());
            }
        }
        let modes = definition
            .airports
            .iter()
            .map(|target| (target.airport, target.runway_modes.clone()))
            .chain(
                definition
                    .edges
                    .iter()
                    .map(|edge| (edge.airport, edge.runway_modes.clone())),
            )
            .filter(|(_, modes)| !modes.is_empty())
            .collect();
        Ok(Self {
            definition,
            by_airport,
            zones,
            modes,
        })
    }

    /// Parses and validates a traffic targets file.
    pub fn from_json(text: &str) -> Result<Self> {
        let definition = serde_json::from_str(text).context("malformed traffic targets")?;
        Self::from_definition(definition).context("invalid traffic targets")
    }

    /// The targets as a definition again.
    pub fn to_definition(&self) -> TrafficDefinition {
        self.definition.clone()
    }

    /// Checks that every listed airport exists in `world` and has the runway
    /// ends its modes name.
    pub fn check_references(&self, world: &World) -> Result<()> {
        let listed = self
            .definition
            .airports
            .iter()
            .map(|t| t.airport)
            .chain(self.definition.edges.iter().map(|e| e.airport));
        for ident in listed {
            let Some(id) = world.airport_by_ident(ident) else {
                return Err(CoreError::UnknownAirport {
                    ident: ident.to_string(),
                }
                .into());
            };
            let ends: Vec<_> = world
                .runways_of(id)
                .iter()
                .flat_map(|runway| {
                    runway
                        .ends()
                        .iter()
                        .map(crate::world::RunwayEnd::designator)
                })
                .collect();
            for mode in self.runway_modes(ident) {
                for runway in mode.arrivals.iter().chain(&mode.departures) {
                    if !ends.contains(runway) {
                        return Err(CoreError::UnknownRunway {
                            airport: ident.to_string(),
                            runway: runway.to_string(),
                        }
                        .into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Relative traffic per weekday, Monday first.
    pub const fn weekday_factors(&self) -> &[f64; 7] {
        &self.definition.weekday_factors
    }

    /// Flights per day crossing the airspace.
    pub const fn overflights_per_day(&self) -> f64 {
        self.definition.overflights_per_day
    }

    /// Coarse outline of the simulated airspace.
    pub fn airspace(&self) -> &[LatLon] {
        &self.definition.airspace
    }

    /// All airport targets in file order.
    pub fn airports(&self) -> impl ExactSizeIterator<Item = &AirportTarget> {
        self.definition.airports.iter()
    }

    /// The target for `airport`, if it is a calibrated commercial airport.
    pub fn airport(&self, airport: AirportIdent) -> Option<&AirportTarget> {
        self.by_airport
            .get(&airport)
            .map(|index| &self.definition.airports[*index])
    }

    /// Runway operating modes of a calibrated or edge airport; empty if none
    /// are curated.
    pub fn runway_modes(&self, airport: AirportIdent) -> &[RunwayMode] {
        self.modes.get(&airport).map_or(&[], Vec::as_slice)
    }

    /// The local time zone of a calibrated or edge airport.
    pub fn zone(&self, airport: AirportIdent) -> Option<&Zone> {
        self.zones.get(&airport)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{CoreError, LatLon, time::Weekday, units::Degrees, world::World};

    fn definition() -> Result<TrafficDefinition> {
        let ring = [
            (50.0, 6.0),
            (50.0, 14.0),
            (54.0, 14.0),
            (54.0, 6.0),
            (50.0, 6.0),
        ];
        let mut airspace = Vec::with_capacity(ring.len());
        for (lat, lon) in ring {
            airspace.push(LatLon::new(lat, lon)?);
        }
        Ok(TrafficDefinition {
            format_version: TRAFFIC_FORMAT_VERSION,
            source: "test fixture".to_owned(),
            weekday_factors: [1.0, 1.0, 1.0, 1.0, 1.2, 0.8, 1.0],
            overflights_per_day: 3500.0,
            airspace,
            airports: vec![AirportTarget {
                airport: "EDDF".parse()?,
                movements_per_year: 365_000,
                zone: Zone::new("Europe/Berlin")?,
                curfew: Some(Curfew {
                    start: "23:00".parse()?,
                    end: "05:00".parse()?,
                }),
                banks: vec!["06:30".parse()?, "08:00".parse()?],
                cargo_share: Some(0.1),
                capacity: Some(Capacity {
                    per_hour: 104,
                    window_minutes: 15,
                    per_window: 30,
                }),
                runway_modes: vec![RunwayMode {
                    flow: Degrees::new(250.0)?,
                    arrivals: vec!["25L".parse()?, "25C".parse()?],
                    departures: vec!["25C".parse()?, "18".parse()?],
                }],
            }],
            edges: vec![EdgeZone {
                airport: "LFPG".parse()?,
                zone: Zone::new("Europe/Paris")?,
                runway_modes: vec![RunwayMode {
                    flow: Degrees::new(270.0)?,
                    arrivals: vec!["27R".parse()?],
                    departures: vec!["27L".parse()?],
                }],
            }],
        })
    }

    fn reason(definition: TrafficDefinition) -> Result<&'static str> {
        let error = TrafficTargets::from_definition(definition)
            .err()
            .context("must be rejected")?;
        match error
            .downcast::<CoreError>()
            .context("expected a CoreError")?
        {
            CoreError::InvalidTraffic { reason } => Ok(reason),
            other => anyhow::bail!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_targets_expose_daily_movements_by_weekday() -> Result<()> {
        let targets = TrafficTargets::from_definition(definition()?)?;
        let frankfurt = targets.airport("EDDF".parse()?).context("EDDF")?;
        assert!(
            (frankfurt.movements_per_day(Weekday::Monday, targets.weekday_factors()) - 1000.0)
                .abs()
                < 1e-9
        );
        assert!(
            (frankfurt.movements_per_day(Weekday::Friday, targets.weekday_factors()) - 1200.0)
                .abs()
                < 1e-9
        );
        assert_eq!(frankfurt.banks().len(), 2);
        assert_eq!(frankfurt.cargo_share(), 0.1);
        assert_eq!(frankfurt.capacity().map(|c| c.per_hour), Some(104));
        assert_eq!(targets.runway_modes("EDDF".parse()?).len(), 1);
        assert_eq!(targets.runway_modes("LFPG".parse()?).len(), 1);
        assert_eq!(targets.runway_modes("EGLL".parse()?).len(), 0);
        assert!(frankfurt.curfew().is_some());
        assert_eq!(
            targets.zone("LFPG".parse()?).map(Zone::name),
            Some("Europe/Paris")
        );
        assert_eq!(
            targets.zone("EDDF".parse()?).map(Zone::name),
            Some("Europe/Berlin")
        );
        assert!(targets.zone("EGLL".parse()?).is_none());
        assert!((targets.overflights_per_day() - 3500.0).abs() < f64::EPSILON);
        assert_eq!(targets.airspace().len(), 5);
        assert_eq!(targets.airports().count(), 1);
        Ok(())
    }

    #[test]
    fn test_targets_reject_inconsistent_definitions() -> Result<()> {
        let mut factors = definition()?;
        factors.weekday_factors[0] = 3.0;
        assert_eq!(
            reason(factors)?,
            "weekday factors must be positive and average 1"
        );

        let mut open = definition()?;
        open.airspace.pop();
        assert_eq!(
            reason(open)?,
            "airspace must be a closed ring of at least 4 points"
        );

        let mut idle = definition()?;
        idle.airports[0].movements_per_year = 0;
        assert_eq!(reason(idle)?, "airport targets must be positive");

        let mut unsorted = definition()?;
        unsorted.airports[0].banks.reverse();
        assert_eq!(reason(unsorted)?, "banks must be strictly increasing");

        let mut repeated = definition()?;
        let copy = repeated.airports[0].clone();
        repeated.airports.push(copy);
        assert_eq!(reason(repeated)?, "each airport may appear only once");

        let mut share = definition()?;
        share.airports[0].cargo_share = Some(1.5);
        assert_eq!(reason(share)?, "cargo share must be within 0..=1");

        let mut window = definition()?;
        if let Some(capacity) = window.airports[0].capacity.as_mut() {
            capacity.window_minutes = 7;
        }
        assert_eq!(reason(window)?, "capacity windows must divide the hour");

        let mut mode = definition()?;
        mode.edges[0].runway_modes[0].arrivals.clear();
        assert_eq!(
            reason(mode)?,
            "runway mode lists must be non-empty and distinct"
        );

        let mut negative = definition()?;
        negative.overflights_per_day = -1.0;
        assert_eq!(
            reason(negative)?,
            "overflights per day must not be negative"
        );
        Ok(())
    }

    #[test]
    fn test_targets_json_round_trip_and_version() -> Result<()> {
        let json = definition()?.to_json_pretty()?;
        assert!(json.contains("\"start\": \"23:00\""), "{json}");
        let targets = TrafficTargets::from_json(&json)?;
        assert_eq!(targets.to_definition(), definition()?);
        let future = json.replacen("\"format_version\": 2", "\"format_version\": 5", 1);
        let error = TrafficTargets::from_json(&future)
            .err()
            .context("must fail")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "traffic targets",
                found: 5,
                supported: 2
            })
        );
        Ok(())
    }

    #[test]
    fn test_shipped_targets_are_valid_and_resolve() -> Result<()> {
        let targets =
            TrafficTargets::from_json(include_str!("../../../../assets/traffic/germany.json"))
                .context("assets/traffic/germany.json")?;
        let world = World::from_json(include_str!("../../../../assets/world/germany.world.json"))?;
        targets.check_references(&world)?;
        let frankfurt = targets.airport("EDDF".parse()?).context("EDDF")?;
        assert_eq!(
            frankfurt.movements_per_year(),
            449_896,
            "ADV 2025, Linien- und Charterverkehr"
        );
        assert!(targets.airports().count() >= 20);
        assert_eq!(frankfurt.capacity().map(|c| c.per_hour), Some(104));
        assert_eq!(targets.runway_modes("EDDF".parse()?).len(), 2);
        for edge in world
            .airports()
            .filter(|(_, a)| a.kind() == crate::world::AirportKind::Edge)
        {
            assert!(
                targets.zone(edge.1.ident()).is_some(),
                "{} has no zone",
                edge.1.ident()
            );
        }
        Ok(())
    }

    #[test]
    fn test_check_references_rejects_unknown_airports() -> Result<()> {
        let targets = TrafficTargets::from_definition(definition()?)?;
        let error = targets
            .check_references(&World::empty())
            .err()
            .context("must fail")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnknownAirport {
                ident: "EDDF".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn test_check_references_rejects_unknown_runways() -> Result<()> {
        let world = World::from_json(include_str!("../../../../assets/world/germany.world.json"))?;
        let mut wrong = definition()?;
        wrong.airports[0].runway_modes[0].departures = vec!["09".parse()?];
        let targets = TrafficTargets::from_definition(wrong)?;
        let error = targets
            .check_references(&world)
            .err()
            .context("must fail")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnknownRunway {
                airport: "EDDF".to_owned(),
                runway: "09".to_owned()
            })
        );
        Ok(())
    }
}
