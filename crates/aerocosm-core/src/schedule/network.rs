//! From calibrated demand to services: which airline flies which route how
//! often.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::{
    FlightKind,
    demand::{DemandParams, Node, Route, fit, gravity},
    trips::{RANGE_MARGIN, RUNWAY_MARGIN},
};
use crate::{
    AircraftCatalog, Id,
    airline::{Airline, AirlineCatalog, AirlineKind},
    traffic::TrafficTargets,
    world::{Airport, World},
};

/// Days per year, to turn yearly movement targets into daily departures.
const DAYS_PER_YEAR: f64 = 365.0;

/// One airline flying round trips from one of its bases to another airport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Service {
    pub(crate) airline: Id<Airline>,
    pub(crate) base: Id<Airport>,
    pub(crate) other: Id<Airport>,
    /// Round trips per average day (weekday factor 1).
    pub(crate) daily: f64,
    pub(crate) kind: FlightKind,
}

/// The airports of one traffic layer (passenger or cargo) as demand nodes.
struct Layer {
    airports: Vec<Id<Airport>>,
    nodes: Vec<Node>,
    /// Each airport's longest runway, m.
    runways: Vec<f64>,
}

/// An airline at one of its bases that belongs to the layer.
struct Candidate {
    airline: Id<Airline>,
    base: usize,
    weight: f64,
    /// What its fleet's types can do: the longest sector each flies and the
    /// runway each needs, m.
    fleet: std::rc::Rc<[(f64, f64)]>,
}

impl Candidate {
    /// Whether a type of its fleet flies `distance` between runways at
    /// least `runway` long.
    fn flies(&self, distance: f64, runway: f64) -> bool {
        self.fleet
            .iter()
            .any(|&(reach, needs)| reach >= distance && needs <= runway)
    }
}

/// Airports with a share of traffic for `kind`, their targets as departures
/// a day; a node's gravity mass is its target, so that big airports pair
/// with big airports.
fn layer(world: &World, traffic: &TrafficTargets, kind: AirlineKind) -> Result<Layer> {
    let mut layer = Layer {
        airports: Vec::new(),
        nodes: Vec::new(),
        runways: Vec::new(),
    };
    for target in traffic.airports() {
        let share = match kind {
            AirlineKind::Passenger => 1.0 - target.cargo_share(),
            AirlineKind::Cargo => target.cargo_share(),
        };
        if share <= 0.0 {
            continue;
        }
        let id = world
            .airport_by_ident(target.airport())
            .with_context(|| format!("target airport {} not in the world", target.airport()))?;
        let airport = world.airport(id).context("airport id out of range")?;
        let departures = f64::from(target.movements_per_year()) / DAYS_PER_YEAR * share / 2.0;
        layer.airports.push(id);
        layer.nodes.push(Node {
            location: airport.location(),
            mass: departures,
            target: Some(departures),
        });
        layer.runways.push(
            world
                .runways_of(id)
                .iter()
                .map(|r| r.length().value())
                .fold(0.0, f64::max),
        );
    }
    Ok(layer)
}

/// The airlines of `kind` at each node of the layer, by node.
fn candidates(
    world: &World,
    aircraft: &AircraftCatalog,
    airlines: &AirlineCatalog,
    layer: &Layer,
    kind: AirlineKind,
) -> Result<Vec<Vec<Candidate>>> {
    let index: HashMap<Id<Airport>, usize> = layer
        .airports
        .iter()
        .enumerate()
        .map(|(i, a)| (*a, i))
        .collect();
    let mut out: Vec<Vec<Candidate>> = (0..layer.nodes.len()).map(|_| Vec::new()).collect();
    for (number, airline) in airlines.iter().enumerate() {
        if airline.kind() != kind {
            continue;
        }
        let id = Id::from_usize(number).context("airline index")?;
        let fleet = capabilities(aircraft, airline);
        for base in airline.bases() {
            let node = world
                .airport_by_ident(base.airport)
                .and_then(|airport| index.get(&airport).copied());
            if let Some(node) = node {
                out[node].push(Candidate {
                    airline: id,
                    base: node,
                    weight: base.weight,
                    fleet: fleet.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// For each type of `airline`'s fleet, the longest sector it flies and the
/// runway it needs, m, with the margins schedule building applies.
fn capabilities(aircraft: &AircraftCatalog, airline: &Airline) -> std::rc::Rc<[(f64, f64)]> {
    airline
        .fleet()
        .iter()
        .filter_map(|share| aircraft.get(aircraft.by_code(share.aircraft)?))
        .map(|a| {
            (
                a.range().to_meters().value() / RANGE_MARGIN,
                a.min_runway_length().value() * RUNWAY_MARGIN,
            )
        })
        .collect()
}

/// The airlines at either end of `route` with a type that flies its
/// `distance` between runways at least `runway` long, with the airport they
/// fly to.
fn eligible<'a>(
    candidates: &'a [Vec<Candidate>],
    route: &Route,
    distance: f64,
    runway: f64,
) -> impl Iterator<Item = (&'a Candidate, usize)> {
    candidates[route.from]
        .iter()
        .map(|c| (c, route.to))
        .chain(candidates[route.to].iter().map(|c| (c, route.from)))
        .filter(move |(c, _)| c.flies(distance, runway))
}

/// Passenger or cargo services fitted to the airports' targets.
fn layer_services(
    world: &World,
    aircraft: &AircraftCatalog,
    airlines: &AirlineCatalog,
    traffic: &TrafficTargets,
    kind: AirlineKind,
    out: &mut Vec<Service>,
) -> Result<()> {
    let mut layer = layer(world, traffic, kind)?;
    let candidates = candidates(world, aircraft, airlines, &layer, kind)?;
    let locations: Vec<crate::LatLon> = layer.nodes.iter().map(|n| n.location).collect();
    let runways = layer.runways.clone();
    let shortest = |route: &Route| runways[route.from].min(runways[route.to]);
    let distance = |route: &Route| {
        locations[route.from]
            .distance_to(locations[route.to])
            .value()
    };
    let mut routes = gravity(&layer.nodes, &DemandParams::default());
    routes.retain(|route| {
        eligible(&candidates, route, distance(route), shortest(route))
            .next()
            .is_some()
    });
    // A node left without a route cannot meet its target: it drops out.
    let mut linked = vec![false; layer.nodes.len()];
    for route in &routes {
        linked[route.from] = true;
        linked[route.to] = true;
    }
    for (node, linked) in layer.nodes.iter_mut().zip(linked) {
        if !linked {
            node.target = None;
        }
    }
    let routes = fit(&layer.nodes, routes).with_context(|| format!("{kind:?} demand"))?;
    let flight_kind = match kind {
        AirlineKind::Passenger => FlightKind::Passenger,
        AirlineKind::Cargo => FlightKind::Cargo,
    };
    for route in routes.into_iter().filter(|r| r.daily > 0.0) {
        let (length, runway) = (distance(&route), shortest(&route));
        let total: f64 = eligible(&candidates, &route, length, runway)
            .map(|(c, _)| c.weight)
            .sum();
        if total <= 0.0 {
            continue;
        }
        for (candidate, other) in eligible(&candidates, &route, length, runway) {
            out.push(Service {
                airline: candidate.airline,
                base: layer.airports[candidate.base],
                other: layer.airports[other],
                daily: route.daily * candidate.weight / total,
                kind: flight_kind,
            });
        }
    }
    Ok(())
}

/// Every service of the week template: passenger and cargo services meeting
/// the airports' targets.
pub(crate) fn plan_services(
    world: &World,
    aircraft: &AircraftCatalog,
    airlines: &AirlineCatalog,
    traffic: &TrafficTargets,
) -> Result<Vec<Service>> {
    let mut services = Vec::new();
    for kind in [AirlineKind::Passenger, AirlineKind::Cargo] {
        layer_services(world, aircraft, airlines, traffic, kind, &mut services)?;
    }
    Ok(services)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        airline::AirlineKind,
        test_data::{Data, data},
    };

    fn services(data: &Data) -> Result<Vec<Service>> {
        plan_services(&data.world, &data.aircraft, &data.airlines, &data.traffic)
    }

    /// Average daily departures per airport implied by services of `kind`.
    fn departures(services: &[Service], kind: FlightKind) -> HashMap<Id<Airport>, f64> {
        let mut out = HashMap::new();
        for service in services.iter().filter(|s| s.kind == kind) {
            *out.entry(service.base).or_insert(0.0) += service.daily;
            *out.entry(service.other).or_insert(0.0) += service.daily;
        }
        out
    }

    /// Airports with routes meet their targets in `kind`'s layer; those no
    /// airline can reach (too short a runway for every type flying near)
    /// drop out, but few and with little traffic.
    fn meets_targets(data: &Data, kind: FlightKind) -> Result<()> {
        let services = services(data)?;
        let departures = departures(&services, kind);
        let (mut wanted_total, mut got_total, mut missing, mut count) = (0.0, 0.0, 0, 0);
        for target in data.traffic.airports() {
            let id = data
                .world
                .airport_by_ident(target.airport())
                .context("airport")?;
            let share = match kind {
                FlightKind::Passenger => 1.0 - target.cargo_share(),
                FlightKind::Cargo => target.cargo_share(),
            };
            let wanted = f64::from(target.movements_per_year()) / 365.0 * share / 2.0;
            if wanted <= 0.0 {
                continue;
            }
            count += 1;
            wanted_total += wanted;
            let got = departures.get(&id).copied().unwrap_or(0.0);
            got_total += got;
            if got == 0.0 {
                missing += 1;
                continue;
            }
            assert!(
                (got - wanted).abs() / wanted < 0.01,
                "{}: {got:.1} vs {wanted:.1}",
                target.airport()
            );
        }
        assert!(missing * 50 < count, "{missing} of {count} without routes");
        assert!(
            got_total >= wanted_total * 0.98,
            "{got_total:.0} of {wanted_total:.0}"
        );
        Ok(())
    }

    #[test]
    fn test_passenger_services_meet_departure_targets() -> Result<()> {
        meets_targets(&data()?, FlightKind::Passenger)
    }

    #[test]
    fn test_cargo_services_meet_cargo_targets_with_cargo_airlines() -> Result<()> {
        let data = data()?;
        meets_targets(&data, FlightKind::Cargo)?;
        for service in services(&data)?
            .iter()
            .filter(|s| s.kind == FlightKind::Cargo)
        {
            let airline = data.airlines.get(service.airline).context("airline")?;
            assert_eq!(airline.kind(), AirlineKind::Cargo, "{}", airline.icao());
        }
        Ok(())
    }

    #[test]
    fn test_services_follow_network_rules() -> Result<()> {
        let data = data()?;
        for service in services(&data)? {
            let airline = data.airlines.get(service.airline).context("airline")?;
            let base = data.world.airport(service.base).context("base")?;
            let other = data.world.airport(service.other).context("other")?;
            assert!(
                airline.bases().iter().any(|b| b.airport == base.ident()),
                "{} not based at {}",
                airline.icao(),
                base.ident()
            );
            assert!(service.daily > 0.0);
            assert_ne!(base.ident(), other.ident());
            assert_eq!(
                airline.kind() == AirlineKind::Cargo,
                service.kind == FlightKind::Cargo
            );
        }
        Ok(())
    }
}
