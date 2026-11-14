//! Route demand: gravity weights between airports, fitted to departure targets
//! (plan D28).

use anyhow::Result;

use crate::{CoreError, LatLon, units::Meters};

/// Iterations of proportional fitting; it converges in a few dozen for real
/// networks.
const MAX_ITERATIONS: usize = 500;
/// Fitting stops once every target is met to this relative error.
const TOLERANCE: f64 = 1e-6;

/// An airport in the demand model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Node {
    pub(crate) location: LatLon,
    /// Attraction, e.g. catchment population.
    pub(crate) mass: f64,
    /// Departures per day to reach; `None` for edge airports, which absorb what
    /// the calibrated airports send.
    pub(crate) target: Option<f64>,
}

/// Demand between two nodes (`from < to`), in flights per day in each
/// direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Route {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) daily: f64,
}

/// Shape of the gravity model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DemandParams {
    /// Distance decay exponent.
    pub(crate) beta: f64,
    /// Pairs closer than this get no direct flights (rail and road win).
    pub(crate) min_distance: Meters,
    /// Each node keeps its strongest partners only (plan M5 D112); a pair
    /// stays when either end keeps the other.
    pub(crate) partners: usize,
}

impl Default for DemandParams {
    fn default() -> Self {
        Self {
            beta: 1.2,
            min_distance: Meters::from_raw(180_000.0),
            partners: 150,
        }
    }
}

/// Mean Earth radius, m, as in the great-circle maths.
const EARTH_RADIUS: f64 = 6_371_008.8;

/// Unscaled gravity weights `m_i · m_j / d^β` for every pair with at least one
/// calibrated end, at least `min_distance` apart, among each node's
/// `partners` strongest.
pub(crate) fn gravity(nodes: &[Node], params: &DemandParams) -> Vec<Route> {
    // Unit vectors: the chord between two points gives their great-circle
    // distance without trigonometry per pair.
    let points: Vec<[f64; 3]> = nodes
        .iter()
        .map(|n| {
            let (lat, lon) = (
                n.location.latitude().value().to_radians(),
                n.location.longitude().value().to_radians(),
            );
            [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
        })
        .collect();
    let min_kilometres = params.min_distance.value() / 1000.0;
    let mut row: Vec<(usize, f64)> = Vec::with_capacity(nodes.len());
    let mut pairs: Vec<(usize, usize, f64)> =
        Vec::with_capacity(nodes.len() * params.partners.min(nodes.len()));
    for (from, a) in nodes.iter().enumerate() {
        row.clear();
        for (to, b) in nodes.iter().enumerate() {
            if to == from || (a.target.is_none() && b.target.is_none()) {
                continue;
            }
            let chord = points[from]
                .iter()
                .zip(&points[to])
                .map(|(p, q)| (p - q).powi(2))
                .sum::<f64>()
                .sqrt();
            let kilometres = 2.0 * EARTH_RADIUS * (chord / 2.0).min(1.0).asin() / 1000.0;
            if kilometres < min_kilometres {
                continue;
            }
            row.push((to, a.mass * b.mass / kilometres.powf(params.beta)));
        }
        if params.partners > 0 && row.len() > params.partners {
            row.select_nth_unstable_by(params.partners - 1, |p, q| q.1.total_cmp(&p.1));
            row.truncate(params.partners);
        }
        pairs.extend(
            row.iter()
                .map(|&(to, daily)| (from.min(to), from.max(to), daily)),
        );
    }
    pairs.sort_unstable_by_key(|p| (p.0, p.1));
    pairs.dedup_by_key(|p| (p.0, p.1));
    pairs
        .into_iter()
        .map(|(from, to, daily)| Route { from, to, daily })
        .collect()
}

/// Scales gravity weights so every calibrated node's departures (the sum of its
/// routes) equal its target. Each calibrated node gets a factor `a_i`, edge
/// nodes keep 1, and a route's demand is `a_i · a_j · w_ij` (symmetric
/// proportional fitting).
pub(crate) fn fit(nodes: &[Node], mut routes: Vec<Route>) -> Result<Vec<Route>> {
    let mut adjacency: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nodes.len()];
    for route in &routes {
        adjacency[route.from].push((route.to, route.daily));
        adjacency[route.to].push((route.from, route.daily));
    }
    let mut factor = vec![1.0_f64; nodes.len()];
    for _ in 0..MAX_ITERATIONS {
        let mut worst = 0.0_f64;
        for (index, node) in nodes.iter().enumerate() {
            let Some(target) = node.target else {
                continue;
            };
            let reach: f64 = adjacency[index]
                .iter()
                .map(|&(other, weight)| factor[other] * weight)
                .sum();
            if reach <= 0.0 {
                return Err(CoreError::ScheduleFailed {
                    reason: format!("node {index} has a target but no routes"),
                }
                .into());
            }
            let wanted = target / reach;
            worst = worst.max((wanted / factor[index] - 1.0).abs());
            factor[index] = wanted;
        }
        if worst < TOLERANCE {
            break;
        }
    }
    for route in &mut routes {
        route.daily *= factor[route.from] * factor[route.to];
    }
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;
    use crate::LatLon;

    fn node(lat: f64, lon: f64, mass: f64, target: Option<f64>) -> Result<Node> {
        Ok(Node {
            location: LatLon::new(lat, lon)?,
            mass,
            target,
        })
    }

    fn departures(routes: &[Route], index: usize) -> f64 {
        routes
            .iter()
            .filter(|r| r.from == index || r.to == index)
            .map(|r| r.daily)
            .sum()
    }

    #[test]
    fn test_gravity_skips_close_and_edge_to_edge_pairs() -> Result<()> {
        let nodes = [
            node(50.03, 8.57, 5e6, Some(100.0))?, // Frankfurt
            node(49.96, 8.65, 1e5, Some(5.0))?,   // Egelsbach, 10 km away
            node(49.0, 2.55, 10e6, None)?,        // Paris
            node(51.47, -0.45, 11e6, None)?,      // London
        ];
        let routes = gravity(&nodes, &DemandParams::default());
        let pairs: Vec<(usize, usize)> = routes.iter().map(|r| (r.from, r.to)).collect();
        assert!(!pairs.contains(&(0, 1)), "too close");
        let distance = nodes[0].location.distance_to(nodes[2].location).value() / 1000.0;
        let weight = routes
            .iter()
            .find(|r| (r.from, r.to) == (0, 2))
            .map_or(0.0, |r| r.daily);
        assert!((weight - 5e6 * 10e6 / distance.powf(1.2)).abs() / weight < 1e-6);
        assert!(
            !pairs.contains(&(2, 3)),
            "edge to edge is not German traffic"
        );
        assert!(pairs.contains(&(0, 2)) && pairs.contains(&(0, 3)) && pairs.contains(&(1, 2)));
        assert!(routes.iter().all(|r| r.from < r.to && r.daily > 0.0));
        Ok(())
    }

    #[test]
    fn test_gravity_keeps_the_strongest_partners() -> Result<()> {
        // A big node and five small ones in a row east of it; with two
        // partners each, the far small ones only pair with their own
        // neighbours or the big node.
        let mut nodes = vec![node(50.0, 0.0, 1e7, Some(100.0))?];
        for k in 1..=5 {
            nodes.push(node(50.0, 3.0 * f64::from(k), 1e5, Some(10.0))?);
        }
        let params = DemandParams {
            partners: 2,
            ..DemandParams::default()
        };
        let routes = gravity(&nodes, &params);
        let pairs: Vec<(usize, usize)> = routes.iter().map(|r| (r.from, r.to)).collect();
        assert!(pairs.contains(&(0, 1)) && pairs.contains(&(0, 2)));
        assert!(!pairs.contains(&(1, 5)), "weak pair dropped: {pairs:?}");
        let mut sorted = pairs.clone();
        sorted.dedup();
        assert_eq!(sorted, pairs, "each pair once");
        Ok(())
    }

    #[test]
    fn test_gravity_prefers_big_and_near() -> Result<()> {
        let nodes = [
            node(50.0, 8.0, 1e6, Some(10.0))?,
            node(52.0, 8.0, 1e6, None)?,
            node(54.0, 8.0, 1e6, None)?,
            node(52.0, 8.0, 4e6, None)?,
        ];
        let routes = gravity(&nodes, &DemandParams::default());
        let weight = |to: usize| {
            routes
                .iter()
                .find(|r| r.from == 0 && r.to == to)
                .map_or(0.0, |r| r.daily)
        };
        assert!(weight(1) > weight(2), "nearer wins");
        assert!(weight(3) > weight(1), "bigger wins");
        Ok(())
    }

    #[test]
    fn test_fit_meets_targets() -> Result<()> {
        let nodes = [
            node(50.03, 8.57, 5e6, Some(600.0))?,
            node(48.35, 11.79, 4e6, Some(450.0))?,
            node(52.36, 13.5, 4e6, Some(250.0))?,
            node(49.0, 2.55, 10e6, None)?,
            node(51.47, -0.45, 11e6, None)?,
        ];
        let routes = fit(&nodes, gravity(&nodes, &DemandParams::default()))?;
        for (index, n) in nodes.iter().enumerate() {
            if let Some(target) = n.target {
                let got = departures(&routes, index);
                assert!(
                    (got - target).abs() / target < 0.01,
                    "node {index}: {got} vs {target}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn test_fit_rejects_targets_one_route_cannot_meet() -> Result<()> {
        // A proptest case: the second node's only route (the edges lie under
        // the minimum distance) is to the first, so both share its demand
        // and cannot meet targets of 1 and 515.
        let nodes = [
            node(47.5, 6.5, 1e5, Some(1.0))?,
            node(52.675_112, 8.857_987, 1e5, Some(514.596))?,
            node(52.636_674, 10.578_287, 1e6, None)?,
            node(51.182_594, 9.610_469, 1e6, None)?,
        ];
        assert!(fit(&nodes, gravity(&nodes, &DemandParams::default())).is_err());
        Ok(())
    }

    #[test]
    fn test_fit_rejects_unreachable_target() -> Result<()> {
        // A constrained airport with no route at all cannot meet a positive
        // target.
        let nodes = [
            node(50.0, 8.0, 1e6, Some(10.0))?,
            node(50.001, 8.001, 1e6, Some(10.0))?,
        ];
        assert!(fit(&nodes, gravity(&nodes, &DemandParams::default())).is_err());
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_fit_meets_random_targets(
            german in prop::collection::vec((47.5_f64..54.5, 6.5_f64..14.5, 1e5_f64..1e7, 1.0_f64..600.0), 1..12),
            edges in prop::collection::vec((42.0_f64..60.0, -6.0_f64..28.0, 1e6_f64..1.2e7), 2..8),
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let mut nodes = Vec::with_capacity(german.len() + edges.len());
            for (lat, lon, mass, target) in &german {
                nodes.push(node(*lat, *lon, *mass, Some(*target)).map_err(fail)?);
            }
            for (lat, lon, mass) in &edges {
                nodes.push(node(*lat, *lon, *mass, None).map_err(fail)?);
            }
            let routes = gravity(&nodes, &DemandParams::default());
            // Every German node needs a route to an edge node, whose demand
            // is free, for its target to be reachable whatever the others'.
            let edge = |i: usize| i >= german.len();
            prop_assume!((0..german.len()).all(|i| routes
                .iter()
                .any(|r| (r.from == i && edge(r.to)) || (r.to == i && edge(r.from)))));
            let routes = fit(&nodes, routes).map_err(fail)?;
            for (index, n) in nodes.iter().enumerate() {
                if let Some(target) = n.target {
                    let got = departures(&routes, index);
                    prop_assert!((got - target).abs() / target < 0.01, "node {}: {} vs {}", index, got, target);
                }
            }
            prop_assert!(routes.iter().all(|r| r.daily.is_finite() && r.daily >= 0.0));
        }
    }
}
