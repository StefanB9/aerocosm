//! Departures per airport (plan M5 D109): observation where coverage is
//! good, a model fitted on it elsewhere, the world total of 2019 overall.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::{
    build::{DAYS, Station},
    regression::{least_squares, predict},
};
use crate::{
    CoreError, Id,
    traffic::observed::ObservedTraffic,
    world::{Airport, AirportKind, World, codes::CountryCode},
};

/// Countries whose receivers saw at least this share of flights end to end
/// are trusted…
const COVERAGE_MIN: f64 = 0.3;
/// …at airports with at least this many airline departures in the year.
const OBSERVED_MIN: f64 = 365.0;
/// Departures a day worldwide in 2019 (ICAO: about 38.3 million scheduled
/// flights).
pub(super) const WORLD_DEPARTURES_PER_DAY: f64 = 105_000.0;
/// Catchments below this are rounded up: an island airport serves more than
/// its census.
const CATCHMENT_FLOOR: f64 = 1_000.0;
/// The world's median-income country stands in where the figure is missing,
/// US dollars a head.
const UNKNOWN_INCOME: f64 = 10_000.0;

/// What calibration found.
pub(super) struct Calibration {
    pub(super) observed_airports: usize,
    pub(super) modelled_airports: usize,
    pub(super) observed_departures: f64,
    pub(super) modelled_departures: f64,
    pub(super) fit_error: f64,
}

/// Sets every station's departures a day.
pub(super) fn calibrate(
    world: &World,
    observed: &ObservedTraffic,
    stations: &mut [Station],
) -> Result<Calibration> {
    let coverage: HashMap<_, f64> = observed
        .coverage
        .iter()
        .map(|c| (c.country, c.share()))
        .collect();
    // Flights the airport's receivers saw, over those seen whole.
    let seen_at: HashMap<_, f64> = observed
        .airports
        .iter()
        .filter(|a| a.departures + a.arrivals > 0)
        .map(|a| {
            let complete = f64::from(a.departures) + f64::from(a.arrivals);
            (a.airport, (complete + f64::from(a.partial)) / complete)
        })
        .collect();
    let mut trusted: Vec<(usize, f64, f64)> = Vec::new();
    let mut modelled = Vec::new();
    for (i, station) in stations.iter().enumerate() {
        let airport = world.airport(station.id).context("airport id")?;
        let seen = station.seen();
        let share = coverage.get(&airport.country()).copied().unwrap_or(0.0);
        if share >= COVERAGE_MIN && seen >= OBSERVED_MIN {
            let ratio = seen_at
                .get(&airport.ident())
                .copied()
                .unwrap_or(1.0 / share);
            trusted.push((i, seen, ratio));
        } else {
            modelled.push((i, features(world, station.id, airport)));
        }
    }
    // The country's coverage sets the level, each airport's own ratio
    // distributes it: the country's figure alone favours airports with
    // well-covered destinations.
    let mut by_country: HashMap<CountryCode, (f64, f64)> = HashMap::new();
    for &(i, seen, ratio) in &trusted {
        let country = world
            .airport(stations[i].id)
            .context("airport id")?
            .country();
        let entry = by_country.entry(country).or_default();
        entry.0 += seen * ratio;
        entry.1 += seen;
    }
    let mut samples = Vec::with_capacity(trusted.len());
    for &(i, seen, ratio) in &trusted {
        let airport = world.airport(stations[i].id).context("airport id")?;
        let country = airport.country();
        let share = coverage.get(&country).copied().unwrap_or(1.0);
        let mean = by_country
            .get(&country)
            .map_or(ratio, |(weighted, seen)| weighted / seen);
        stations[i].departures = seen * ratio / (share * mean) / DAYS;
        samples.push((
            features(world, stations[i].id, airport),
            stations[i].departures.ln(),
        ));
    }
    let beta = least_squares(&samples).ok_or_else(|| CoreError::ScheduleFailed {
        reason: "too few observed airports to fit the traffic model".to_owned(),
    })?;
    // Duan's smearing: the mean of exp(residual) undoes the bias of fitting
    // in logarithms.
    #[allow(clippy::cast_precision_loss)]
    let count = samples.len() as f64;
    let residuals: Vec<f64> = samples.iter().map(|(x, y)| y - predict(&beta, x)).collect();
    let smearing = residuals.iter().map(|r| r.exp()).sum::<f64>() / count;
    let fit_error = (residuals.iter().map(|r| r * r).sum::<f64>() / count).sqrt();
    let observed_departures: f64 = stations.iter().map(|s| s.departures).sum();
    let raw: Vec<(usize, f64)> = modelled
        .iter()
        .map(|(i, x)| (*i, predict(&beta, x).exp() * smearing))
        .collect();
    let model_total: f64 = raw.iter().map(|(_, d)| d).sum();
    let modelled_departures = WORLD_DEPARTURES_PER_DAY - observed_departures;
    if modelled_departures <= 0.0 || model_total <= 0.0 {
        return Err(CoreError::ScheduleFailed {
            reason: "observed traffic exceeds the world total".to_owned(),
        }
        .into());
    }
    // The model fills what the world total leaves.
    let scale = modelled_departures / model_total;
    for (i, departures) in &raw {
        stations[*i].departures = departures * scale;
    }
    Ok(Calibration {
        observed_airports: trusted.len(),
        modelled_airports: raw.len(),
        observed_departures,
        modelled_departures,
        fit_error,
    })
}

/// Regressors of the fallback model: a constant, the logarithms of the
/// catchment, the country's GDP per head, the longest runway and the runway
/// count, and whether the airport is a major one.
fn features(world: &World, id: Id<Airport>, airport: &Airport) -> Vec<f64> {
    let income = world
        .country(airport.country())
        .map_or(UNKNOWN_INCOME, crate::world::Country::gdp_per_capita);
    let runways = world.runways_of(id);
    let longest = runways
        .iter()
        .map(|r| r.length().value())
        .fold(500.0_f64, f64::max);
    // Runway counts are tiny.
    #[allow(clippy::cast_precision_loss)]
    let count = runways.len() as f64;
    vec![
        1.0,
        airport
            .catchment()
            .map_or(CATCHMENT_FLOOR, f64::from)
            .max(CATCHMENT_FLOOR)
            .ln(),
        income.ln(),
        longest.ln(),
        count.ln_1p(),
        if airport.kind() == AirportKind::Major {
            1.0
        } else {
            0.0
        },
    ]
}
