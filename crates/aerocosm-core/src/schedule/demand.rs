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
}

impl Default for DemandParams {
    fn default() -> Self {
        Self {
            beta: 1.2,
            min_distance: Meters::from_raw(180_000.0),
        }
    }
}

/// Unscaled gravity weights `m_i · m_j / d^β` for every pair with at least one
/// calibrated end, at least `min_distance` apart.
pub(crate) fn gravity(nodes: &[Node], params: &DemandParams) -> Vec<Route> {
    let mut routes = Vec::with_capacity(nodes.len() * nodes.len() / 2);
    for (from, a) in nodes.iter().enumerate() {
        for (offset, b) in nodes[from + 1..].iter().enumerate() {
            if a.target.is_none() && b.target.is_none() {
                continue;
            }
            let distance = a.location.distance_to(b.location).value();
            if distance < params.min_distance.value() {
                continue;
            }
            let kilometres = distance / 1000.0;
            routes.push(Route {
                from,
                to: from + 1 + offset,
                daily: a.mass * b.mass / kilometres.powf(params.beta),
            });
        }
    }
    routes
}

/// Scales gravity weights so every calibrated node's departures (the sum of its
/// routes) equal its target. Each calibrated node gets a factor `a_i`, edge
/// nodes keep 1, and a route's demand is `a_i · a_j · w_ij` (symmetric
/// proportional fitting).
pub(crate) fn fit(nodes: &[Node], mut routes: Vec<Route>) -> Result<Vec<Route>> {
    let mut factor = vec![1.0_f64; nodes.len()];
    for _ in 0..MAX_ITERATIONS {
        let mut worst = 0.0_f64;
        for (index, node) in nodes.iter().enumerate() {
            let Some(target) = node.target else {
                continue;
            };
            let reach: f64 = routes
                .iter()
                .filter_map(|r| match (r.from == index, r.to == index) {
                    (true, _) => Some(factor[r.to] * r.daily),
                    (_, true) => Some(factor[r.from] * r.daily),
                    _ => None,
                })
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
        assert!(
            !pairs.contains(&(2, 3)),
            "edge to edge is not German traffic"
        );
        assert!(pairs.contains(&(0, 2)) && pairs.contains(&(0, 3)) && pairs.contains(&(1, 2)));
        assert!(routes.iter().all(|r| r.from < r.to && r.daily > 0.0));
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
            // Every German node needs at least one route for its target to be reachable.
            prop_assume!((0..german.len()).all(|i| routes.iter().any(|r| r.from == i || r.to == i)));
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
