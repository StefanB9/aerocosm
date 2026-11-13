//! The world model (plan M5 D109–D113): movement targets, airlines, runway
//! use and banks for every scheduled airport, derived by general rules from
//! the world and the traffic observed in 2019. Nothing in it is curated per
//! airport or per airline (D95).

mod airlines;
mod build;
mod calibrate;
mod regression;
mod runways;

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use self::{
    build::{Carrier, DAYS, Station, bases, carriers, inherit, stations},
    calibrate::calibrate,
    runways::runway_use,
};
use super::{
    AirportTarget, ClockTime, TrafficDefinition, TrafficTargets, observed::ObservedTraffic,
};
use crate::{
    AircraftCatalog,
    airline::{AIRLINE_FORMAT_VERSION, AirlineCatalog, AirlineCatalogDefinition, AirlineKind},
    world::World,
};

/// Hub carriers get one bank per this many departures a day at a hub…
const DEPARTURES_PER_BANK: f64 = 60.0;
/// …between these many banks…
const BANKS: (f64, f64) = (3.0, 8.0);
/// …spread evenly over this local window, minutes after midnight.
const BANK_WINDOW: (f64, f64) = (6.0 * 60.0, 22.0 * 60.0);
/// An airport is coordinated (slots, plan M3 D54) when its peak hour, this
/// share of the day's movements, exceeds this share of its capacity.
const PEAK_HOUR_SHARE: f64 = 0.075;
const COORDINATION_LOAD: f64 = 0.8;
/// Relative traffic per weekday, Monday first: one general pattern.
const WEEKDAY_FACTORS: [f64; 7] = [1.03, 0.97, 0.98, 1.02, 1.06, 0.88, 1.06];

/// What the rules made of the world.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldModel {
    targets: TrafficTargets,
    airlines: AirlineCatalog,
    report: ModelReport,
}

impl WorldModel {
    /// Movement targets, runway use and banks per scheduled airport.
    pub const fn targets(&self) -> &TrafficTargets {
        &self.targets
    }

    /// The airlines.
    pub const fn airlines(&self) -> &AirlineCatalog {
        &self.airlines
    }

    /// How the model was built.
    pub const fn report(&self) -> &ModelReport {
        &self.report
    }

    /// Targets and airlines, for a [`crate::Scenario`].
    pub fn into_parts(self) -> (TrafficTargets, AirlineCatalog) {
        (self.targets, self.airlines)
    }
}

/// Figures from building the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelReport {
    /// Airports whose target comes from observation.
    pub observed_airports: usize,
    /// Airports whose target comes from the fallback model.
    pub modelled_airports: usize,
    /// Departures a day at the observed airports.
    pub observed_departures: f64,
    /// Departures a day at the modelled airports, after scaling.
    pub modelled_departures: f64,
    /// Passenger and cargo airlines.
    pub airlines: usize,
    /// Of which cargo.
    pub cargo_airlines: usize,
    /// Callsign prefixes dropped as not airlines.
    pub dropped_airlines: usize,
    /// Root mean square error of the fallback fit in logarithms.
    pub fit_error: f64,
}

/// Builds the world model from `world`, the `observed` traffic and the
/// `aircraft` catalog.
pub fn derive(
    world: &World,
    observed: &ObservedTraffic,
    aircraft: &AircraftCatalog,
) -> Result<WorldModel> {
    let mut stations = stations(world);
    let (mut carriers, dropped) = carriers(world, observed, aircraft, &mut stations);
    let calibration = calibrate(world, observed, &mut stations)?;
    inherit(world, aircraft, &mut stations, &mut carriers)?;
    let hubs = bases(world, &stations, &mut carriers)?;
    let targets = targets(world, &stations, &carriers, &hubs)?;
    let report = ModelReport {
        observed_airports: calibration.observed_airports,
        modelled_airports: calibration.modelled_airports,
        observed_departures: calibration.observed_departures,
        modelled_departures: calibration.modelled_departures,
        airlines: carriers.len(),
        cargo_airlines: carriers
            .iter()
            .filter(|c| c.definition.kind == AirlineKind::Cargo)
            .count(),
        dropped_airlines: dropped,
        fit_error: calibration.fit_error,
    };
    let targets = TrafficTargets::from_definition(TrafficDefinition {
        source: format!(
            "Derived by rule (plan M5 D109-D113) from the OpenSky {} flight lists, OurAirports, \
             Natural Earth and NASA POWER",
            observed.year
        ),
        weekday_factors: WEEKDAY_FACTORS,
        airports: targets,
    })
    .context("derived traffic targets")?;
    let airlines = AirlineCatalog::from_definition(AirlineCatalogDefinition {
        format_version: AIRLINE_FORMAT_VERSION,
        airlines: carriers.into_iter().map(|c| c.definition).collect(),
    })
    .context("derived airlines")?;
    Ok(WorldModel {
        targets,
        airlines,
        report,
    })
}

/// Each station's target: its movements, cargo share, runway use, capacity
/// where coordinated, and the banks of its hub carriers.
fn targets(
    world: &World,
    stations: &[Station],
    carriers: &[Carrier],
    hubs: &BTreeMap<usize, f64>,
) -> Result<Vec<AirportTarget>> {
    let mut targets = Vec::with_capacity(stations.len());
    for (i, station) in stations.iter().enumerate() {
        let airport = world.airport(station.id).context("airport id")?;
        let Some(zone) = airport.zone().cloned() else {
            continue;
        };
        let at = station.seen();
        let cargo: f64 = station
            .airlines
            .iter()
            .filter(|(c, _)| carriers[**c].definition.kind == AirlineKind::Cargo)
            .map(|(_, n)| n)
            .sum();
        let movements = 2.0 * station.departures;
        let used = airport
            .wind()
            .and_then(|wind| runway_use(world.runways_of(station.id), wind));
        let (runway_modes, capacity) = match used {
            Some(used) => {
                let coordinated = movements * PEAK_HOUR_SHARE
                    > COORDINATION_LOAD * f64::from(used.capacity.per_hour);
                (used.modes, coordinated.then_some(used.capacity))
            }
            None => (Vec::new(), None),
        };
        // SAFETY: a day's movements times 365 is far below u32::MAX.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let movements_per_year = (movements * DAYS).round().max(1.0) as u32;
        targets.push(AirportTarget {
            airport: airport.ident(),
            movements_per_year,
            zone,
            curfew: None,
            banks: hubs.get(&i).map_or_else(Vec::new, |d| bank_times(*d)),
            cargo_share: (at > 0.0 && cargo > 0.0).then_some(cargo / at),
            capacity,
            runway_modes,
        });
    }
    Ok(targets)
}

/// Bank times for a hub carrier flying `daily` departures a day from a hub.
fn bank_times(daily: f64) -> Vec<ClockTime> {
    let count = (daily / DEPARTURES_PER_BANK)
        .round()
        .clamp(BANKS.0, BANKS.1);
    let spacing = (BANK_WINDOW.1 - BANK_WINDOW.0) / count;
    // Up to eight banks within the day.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (0..count as u16)
        .filter_map(|k| {
            let minute = BANK_WINDOW.0 + (f64::from(k) + 0.5) * spacing;
            ClockTime::from_minute(minute.round() as u16).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests;
