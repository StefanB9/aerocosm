//! From calibrated demand to services: which airline flies which route how
//! often.

use anyhow::{Context, Result};

use super::{
    FlightKind,
    demand::{DemandParams, Node, Route, fit, gravity},
    overflight::crosses,
};
use crate::{
    Id,
    airline::{Airline, AirlineCatalog, AirlineKind},
    traffic::TrafficTargets,
    world::{Airport, AirportKind, World},
};

/// Gravity mass for an airport without a catchment figure.
const FALLBACK_MASS: f64 = 100_000.0;
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
    edge: Vec<bool>,
}

impl Layer {
    fn index(&self, airport: Id<Airport>) -> Option<usize> {
        self.airports.iter().position(|a| *a == airport)
    }
}

/// An airline at one of its bases that belongs to the layer.
struct Candidate {
    airline: Id<Airline>,
    base: usize,
    weight: f64,
}

fn mass(airport: &Airport) -> f64 {
    airport.catchment().map_or(FALLBACK_MASS, f64::from)
}

/// Calibrated airports with their share of traffic for `kind`, plus every edge
/// airport.
fn layer(world: &World, traffic: &TrafficTargets, kind: AirlineKind) -> Result<Layer> {
    let mut layer = Layer {
        airports: Vec::new(),
        nodes: Vec::new(),
        edge: Vec::new(),
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
        layer.airports.push(id);
        layer.nodes.push(Node {
            location: airport.location(),
            mass: mass(airport),
            target: Some(f64::from(target.movements_per_year()) / DAYS_PER_YEAR * share / 2.0),
        });
        layer.edge.push(false);
    }
    for (id, airport) in world.airports() {
        if airport.kind() == AirportKind::Edge {
            layer.airports.push(id);
            layer.nodes.push(Node {
                location: airport.location(),
                mass: mass(airport),
                target: None,
            });
            layer.edge.push(true);
        }
    }
    Ok(layer)
}

fn candidates(
    world: &World,
    airlines: &AirlineCatalog,
    layer: &Layer,
    kind: AirlineKind,
) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for (index, airline) in airlines.iter().enumerate() {
        if airline.kind() != kind {
            continue;
        }
        let id = Id::from_usize(index).context("airline index")?;
        for base in airline.bases() {
            let node = world
                .airport_by_ident(base.airport)
                .and_then(|airport| layer.index(airport));
            if let Some(base_node) = node {
                out.push(Candidate {
                    airline: id,
                    base: base_node,
                    weight: base.weight,
                });
            }
        }
    }
    Ok(out)
}

/// Network rules: a carrier based at an edge airport only flies to German
/// airports, except on overflights, which are edge to edge; a carrier based in
/// Germany may fly anywhere.
fn may_serve(layer: &Layer, base: usize, other: usize, overflight: bool) -> bool {
    match (overflight, layer.edge[base], layer.edge[other]) {
        (true, base_edge, other_edge) => base_edge && other_edge,
        (false, true, other_edge) => !other_edge,
        (false, false, _) => base != other,
    }
}

/// Splits a route's daily round trips between the airlines that may fly it, in
/// proportion to their market share at their base end.
fn split(
    layer: &Layer,
    candidates: &[Candidate],
    route: Route,
    kind: FlightKind,
    out: &mut Vec<Service>,
) {
    let overflight = kind == FlightKind::Overflight;
    let eligible = || {
        candidates.iter().filter_map(move |c| {
            let other = if c.base == route.from {
                route.to
            } else if c.base == route.to {
                route.from
            } else {
                return None;
            };
            may_serve(layer, c.base, other, overflight).then_some((c, other))
        })
    };
    let total: f64 = eligible().map(|(c, _)| c.weight).sum();
    if total <= 0.0 {
        return;
    }
    for (candidate, other) in eligible() {
        out.push(Service {
            airline: candidate.airline,
            base: layer.airports[candidate.base],
            other: layer.airports[other],
            daily: route.daily * candidate.weight / total,
            kind,
        });
    }
}

fn servable(layer: &Layer, candidates: &[Candidate], route: &Route, overflight: bool) -> bool {
    candidates.iter().any(|c| {
        (c.base == route.from && may_serve(layer, c.base, route.to, overflight))
            || (c.base == route.to && may_serve(layer, c.base, route.from, overflight))
    })
}

/// Passenger or cargo services fitted to the airports' targets.
fn layer_services(
    world: &World,
    airlines: &AirlineCatalog,
    traffic: &TrafficTargets,
    kind: AirlineKind,
    out: &mut Vec<Service>,
) -> Result<()> {
    let layer = layer(world, traffic, kind)?;
    let candidates = candidates(world, airlines, &layer, kind)?;
    let mut routes = gravity(&layer.nodes, &DemandParams::default());
    routes.retain(|route| servable(&layer, &candidates, route, false));
    let routes = fit(&layer.nodes, routes).with_context(|| format!("{kind:?} demand"))?;
    let flight_kind = match kind {
        AirlineKind::Passenger => FlightKind::Passenger,
        AirlineKind::Cargo => FlightKind::Cargo,
    };
    for route in routes.into_iter().filter(|r| r.daily > 0.0) {
        split(&layer, &candidates, route, flight_kind, out);
    }
    Ok(())
}

/// Edge-to-edge routes across the airspace, gravity-weighted and scaled to the
/// target.
fn overflight_services(
    world: &World,
    airlines: &AirlineCatalog,
    traffic: &TrafficTargets,
    out: &mut Vec<Service>,
) -> Result<()> {
    let layer = layer(world, traffic, AirlineKind::Passenger)?;
    let candidates = candidates(world, airlines, &layer, AirlineKind::Passenger)?;
    let params = DemandParams::default();
    let mut routes = Vec::new();
    for from in 0..layer.nodes.len() {
        for to in from + 1..layer.nodes.len() {
            if !(layer.edge[from] && layer.edge[to]) {
                continue;
            }
            let (a, b) = (layer.nodes[from], layer.nodes[to]);
            let route = Route {
                from,
                to,
                daily: a.mass * b.mass
                    / (a.location.distance_to(b.location).value() / 1000.0).powf(params.beta),
            };
            if servable(&layer, &candidates, &route, true)
                && crosses(traffic.airspace(), a.location, b.location)
            {
                routes.push(route);
            }
        }
    }
    let total: f64 = routes.iter().map(|r| 2.0 * r.daily).sum();
    if total <= 0.0 {
        return Ok(());
    }
    let scale = traffic.overflights_per_day() / total;
    for mut route in routes {
        route.daily *= scale;
        split(&layer, &candidates, route, FlightKind::Overflight, out);
    }
    Ok(())
}

/// Every service of the week template: passenger and cargo services meeting the
/// airports' targets, and overflights meeting the overflight target.
pub(crate) fn plan_services(
    world: &World,
    airlines: &AirlineCatalog,
    traffic: &TrafficTargets,
) -> Result<Vec<Service>> {
    let mut services = Vec::new();
    layer_services(
        world,
        airlines,
        traffic,
        AirlineKind::Passenger,
        &mut services,
    )?;
    layer_services(world, airlines, traffic, AirlineKind::Cargo, &mut services)?;
    overflight_services(world, airlines, traffic, &mut services)?;
    Ok(services)
}

#[cfg(test)]
pub(super) mod tests {
    use std::collections::HashMap;

    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        AircraftCatalog,
        airline::{AirlineCatalog, AirlineKind},
        traffic::TrafficTargets,
        world::{AirportKind, World},
    };

    pub(in crate::schedule) struct Data {
        pub(in crate::schedule) world: World,
        pub(in crate::schedule) aircraft: AircraftCatalog,
        pub(in crate::schedule) airlines: AirlineCatalog,
        pub(in crate::schedule) traffic: TrafficTargets,
    }

    pub(in crate::schedule) fn data() -> Result<Data> {
        Ok(Data {
            world: World::from_json(include_str!("../../../../assets/world/germany.world.json"))?,
            aircraft: AircraftCatalog::from_json(include_str!(
                "../../../../assets/aircraft/types.json"
            ))?,
            airlines: AirlineCatalog::from_json(include_str!(
                "../../../../assets/airlines/airlines.json"
            ))?,
            traffic: TrafficTargets::from_json(include_str!(
                "../../../../assets/traffic/germany.json"
            ))?,
        })
    }

    fn services(data: &Data) -> Result<Vec<Service>> {
        plan_services(&data.world, &data.airlines, &data.traffic)
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

    #[test]
    fn test_passenger_services_meet_departure_targets() -> Result<()> {
        let data = data()?;
        let services = services(&data)?;
        let departures = departures(&services, FlightKind::Passenger);
        for target in data.traffic.airports() {
            let id = data
                .world
                .airport_by_ident(target.airport())
                .context("airport")?;
            let wanted =
                f64::from(target.movements_per_year()) / 365.0 * (1.0 - target.cargo_share()) / 2.0;
            let got = departures.get(&id).copied().unwrap_or(0.0);
            assert!(
                (got - wanted).abs() / wanted < 0.01,
                "{}: {got:.1} vs {wanted:.1}",
                target.airport()
            );
        }
        Ok(())
    }

    #[test]
    fn test_cargo_services_meet_cargo_targets_with_cargo_airlines() -> Result<()> {
        let data = data()?;
        let services = services(&data)?;
        let departures = departures(&services, FlightKind::Cargo);
        for target in data.traffic.airports().filter(|t| t.cargo_share() > 0.0) {
            let id = data
                .world
                .airport_by_ident(target.airport())
                .context("airport")?;
            let wanted =
                f64::from(target.movements_per_year()) / 365.0 * target.cargo_share() / 2.0;
            let got = departures.get(&id).copied().unwrap_or(0.0);
            assert!(
                (got - wanted).abs() / wanted < 0.01,
                "{}: {got:.1} vs {wanted:.1}",
                target.airport()
            );
        }
        for service in services.iter().filter(|s| s.kind == FlightKind::Cargo) {
            let airline = data.airlines.get(service.airline).context("airline")?;
            assert_eq!(airline.kind(), AirlineKind::Cargo, "{}", airline.icao());
        }
        Ok(())
    }

    #[test]
    fn test_overflights_meet_target_between_edges_crossing_germany() -> Result<()> {
        let data = data()?;
        let services = services(&data)?;
        let overflights: Vec<&Service> = services
            .iter()
            .filter(|s| s.kind == FlightKind::Overflight)
            .collect();
        let per_day: f64 = overflights.iter().map(|s| 2.0 * s.daily).sum();
        let wanted = data.traffic.overflights_per_day();
        assert!(
            (per_day - wanted).abs() / wanted < 0.01,
            "{per_day:.0} vs {wanted:.0}"
        );
        for service in overflights {
            let [a, b] = [service.base, service.other].map(|id| data.world.airport(id));
            let (a, b) = (a.context("a")?, b.context("b")?);
            assert_eq!((a.kind(), b.kind()), (AirportKind::Edge, AirportKind::Edge));
            assert!(super::super::overflight::crosses(
                data.traffic.airspace(),
                a.location(),
                b.location()
            ));
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
            if service.kind != FlightKind::Overflight && base.kind() == AirportKind::Edge {
                assert_ne!(
                    other.kind(),
                    AirportKind::Edge,
                    "{} flies {}-{}",
                    airline.icao(),
                    base.ident(),
                    other.ident()
                );
            }
            assert_eq!(
                airline.kind() == AirlineKind::Cargo,
                service.kind == FlightKind::Cargo
            );
        }
        Ok(())
    }
}
