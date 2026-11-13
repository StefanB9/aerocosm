//! The world model's working state: scheduled airports as stations,
//! observed airlines as carriers, and the rules that tie them together
//! (plan M5 D110).

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};

use super::airlines::{Verdict, classify, colour, is_hub};
use crate::{
    AircraftCatalog, Id, LatLon,
    airline::{AirlineDefinition, AirlineKind, Base, Network},
    schedule::RUNWAY_MARGIN,
    traffic::observed::ObservedTraffic,
    units::Meters,
    world::{Airport, World, codes::AirportIdent},
};

/// Days per year.
pub(super) const DAYS: f64 = 365.0;
/// Traffic observed at an airport without scheduled service (closed ones
/// moved when the traffic was aggregated) counts at the nearest one within
/// this distance.
const MOVED_WITHIN: Meters = Meters::from_raw(50_000.0);
/// An airline's airport counts as a station from this many departures a
/// year (weekly service).
const STATION_MIN: f64 = 52.0;

/// One scheduled airport while the model is built.
pub(super) struct Station {
    pub(super) id: Id<Airport>,
    /// Departures a year per carrier index (ordered, so that sums are the
    /// same on every run).
    pub(super) airlines: BTreeMap<usize, f64>,
    /// Departures a day: the target.
    pub(super) departures: f64,
}

impl Station {
    /// Airline departures a year.
    pub(super) fn seen(&self) -> f64 {
        self.airlines.values().sum()
    }
}

/// One airline while the model is built.
pub(super) struct Carrier {
    pub(super) definition: AirlineDefinition,
    /// Departures a year per station index.
    pub(super) departures: BTreeMap<usize, f64>,
}

/// Every airport of `world` with scheduled service and a time zone.
pub(super) fn stations(world: &World) -> Vec<Station> {
    world
        .airports()
        .filter(|(_, a)| a.scheduled() && a.zone().is_some())
        .map(|(id, _)| Station {
            id,
            airlines: BTreeMap::new(),
            departures: 0.0,
        })
        .collect()
}

/// The observed airlines the rules keep, with their departures per station
/// (also entered at the stations), and how many prefixes were dropped.
pub(super) fn carriers(
    world: &World,
    observed: &ObservedTraffic,
    aircraft: &AircraftCatalog,
    stations: &mut [Station],
) -> (Vec<Carrier>, usize) {
    let index: HashMap<Id<Airport>, usize> = stations
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let mut moved: HashMap<AirportIdent, Option<usize>> = HashMap::new();
    let mut station_of = |ident| {
        *moved.entry(ident).or_insert_with(|| {
            let id = world.airport_by_ident(ident)?;
            if let Some(i) = index.get(&id) {
                return Some(*i);
            }
            let at = world.airport(id)?.location();
            nearest(world, stations, |_| true, at)
                .filter(|(_, d)| *d <= MOVED_WITHIN.value())
                .map(|(i, _)| i)
        })
    };
    let mut carriers: Vec<Carrier> = Vec::new();
    let mut dropped = 0;
    for airline in &observed.airlines {
        let Verdict::Airline(kind, fleet) = classify(airline, aircraft) else {
            dropped += 1;
            continue;
        };
        let mut departures: BTreeMap<usize, f64> = BTreeMap::new();
        for (ident, flights) in &airline.departures {
            if let Some(i) = station_of(*ident) {
                *departures.entry(i).or_default() += f64::from(*flights);
            }
        }
        departures.retain(|_, n| *n >= STATION_MIN);
        if departures.is_empty() {
            dropped += 1;
            continue;
        }
        carriers.push(Carrier {
            definition: AirlineDefinition {
                icao: airline.icao,
                iata: None,
                name: airline.name.clone(),
                callsign: airline
                    .callsign
                    .clone()
                    .unwrap_or_else(|| airline.name.to_uppercase()),
                kind,
                network: Network::PointToPoint,
                bases: Vec::new(),
                fleet,
                colour: colour(airline.icao.as_str()),
                source: format!(
                    "Derived by rule (plan M5 D110) from the OpenSky {} flight lists; name: \
                     OpenFlights",
                    observed.year
                ),
            },
            departures,
        });
    }
    for (c, carrier) in carriers.iter().enumerate() {
        for (&i, &n) in &carrier.departures {
            stations[i].airlines.insert(c, n);
        }
    }
    (carriers, dropped)
}

/// Airports where no airline was seen that can use their runways take the
/// airlines that can from the nearest airport that has some, in the same
/// country where there is one, in that airport's proportions scaled to
/// their own target.
pub(super) fn inherit(
    world: &World,
    aircraft: &AircraftCatalog,
    stations: &mut [Station],
    carriers: &mut [Carrier],
) -> Result<()> {
    let runway: Vec<f64> = stations
        .iter()
        .map(|s| {
            world
                .runways_of(s.id)
                .iter()
                .map(|r| r.length().value())
                .fold(0.0, f64::max)
        })
        .collect();
    // The shortest runway each carrier's fleet can use.
    let needs: Vec<f64> = carriers
        .iter()
        .map(|carrier| {
            carrier
                .definition
                .fleet
                .iter()
                .filter_map(|share| aircraft.get(aircraft.by_code(share.aircraft)?))
                .map(|a| a.min_runway_length().value() * RUNWAY_MARGIN)
                .fold(f64::INFINITY, f64::min)
        })
        .collect();
    let fits = |carrier: usize, length: f64| needs[carrier] <= length;
    let bare: Vec<usize> = (0..stations.len())
        .filter(|&i| !stations[i].airlines.keys().any(|&c| fits(c, runway[i])))
        .collect();
    for i in bare {
        let airport = world.airport(stations[i].id).context("airport id")?;
        let country = airport.country();
        let length = runway[i];
        let able = |s: &Station| s.airlines.keys().any(|&c| fits(c, length));
        let donor = nearest(
            world,
            stations,
            |s| able(s) && world.airport(s.id).is_some_and(|a| a.country() == country),
            airport.location(),
        )
        .or_else(|| nearest(world, stations, able, airport.location()))
        .map(|(d, _)| d);
        let Some(donor) = donor else {
            continue;
        };
        let mix: Vec<(usize, f64)> = stations[donor]
            .airlines
            .iter()
            .filter(|&(&c, _)| fits(c, length))
            .map(|(&c, &n)| (c, n))
            .collect();
        let total: f64 = mix.iter().map(|(_, n)| n).sum();
        let yearly = stations[i].departures * DAYS;
        for (c, n) in mix {
            stations[i].airlines.insert(c, n / total * yearly);
            carriers[c].departures.insert(i, n / total * yearly);
        }
    }
    Ok(())
}

/// Each carrier's bases, weighted by its share of the airport's departures,
/// and its hubs; returns, per hub station, the most departures a day a hub
/// carrier flies there (which sets its banks).
pub(super) fn bases(
    world: &World,
    stations: &[Station],
    carriers: &mut [Carrier],
) -> Result<BTreeMap<usize, f64>> {
    let mut hubs: BTreeMap<usize, f64> = BTreeMap::new();
    for carrier in carriers {
        let all: f64 = carrier.departures.values().sum();
        for (&i, &own) in &carrier.departures {
            let at = stations[i].seen();
            let ident = world.airport(stations[i].id).context("airport id")?.ident();
            carrier.definition.bases.push(Base {
                airport: ident,
                weight: own / at,
            });
            if carrier.definition.kind == AirlineKind::Passenger && is_hub(own, all, at) {
                carrier.definition.network = Network::Hub;
                let daily = own / at * stations[i].departures;
                let entry = hubs.entry(i).or_default();
                *entry = entry.max(daily);
            }
        }
    }
    Ok(hubs)
}

/// The station nearest `at` among those `keep` accepts, with its distance in
/// metres.
pub(super) fn nearest(
    world: &World,
    stations: &[Station],
    keep: impl Fn(&Station) -> bool,
    at: LatLon,
) -> Option<(usize, f64)> {
    stations
        .iter()
        .enumerate()
        .filter(|(_, s)| keep(s))
        .filter_map(|(i, s)| Some((i, world.airport(s.id)?.location().distance_to(at).value())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}
