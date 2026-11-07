//! Where the network meets runways (plan M4 D77) and where arrivals can wait
//! for a stand (D76).

use super::{AirportGround, Edge, EdgeKind, Node};
use crate::{Id, LatLon, world::Runway};

/// Nodes within this distance of a runway centreline are on the runway, m.
const ON_RUNWAY: f64 = 30.0;
/// A waiting spot keeps this far from every edge it is not on, so aircraft
/// taxi past one waiting there (half a code C wingspan, its clearance and a
/// passing aircraft's half span), m.
const SPOT_CLEARANCE: f64 = 45.0;

/// Distance from `point` to the segment from `start` to `end`, in the flat
/// frame.
fn segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let along = [end[0] - start[0], end[1] - start[1]];
    let squared = along[0].mul_add(along[0], along[1] * along[1]);
    let fraction = if squared > 0.0 {
        ((point[0] - start[0]).mul_add(along[0], (point[1] - start[1]) * along[1]) / squared)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point[0] - fraction.mul_add(along[0], start[0]))
        .hypot(point[1] - fraction.mul_add(along[1], start[1]))
}

impl AirportGround {
    /// `point` in the flat frame of the node positions.
    fn local(&self, point: LatLon) -> [f64; 2] {
        let [lat0, lon0] = self.origin;
        [
            (point.longitude().value() - lon0) * self.east,
            (point.latitude().value() - lat0) * super::airport::METRES_PER_DEGREE,
        ]
    }

    /// The nodes on `runway`'s centreline between its thresholds.
    pub fn nodes_on(&self, runway: &Runway) -> Vec<Id<Node>> {
        let [a, b] = runway.ends().map(|end| self.local(end.threshold()));
        let length = (b[0] - a[0]).hypot(b[1] - a[1]).max(f64::EPSILON);
        let unit = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        let mut nodes = Vec::new();
        for (index, node) in self.nodes().iter().enumerate() {
            let offset = [node.xy[0] - a[0], node.xy[1] - a[1]];
            let along = offset[0] * unit[0] + offset[1] * unit[1];
            let off = (offset[0] * unit[1] - offset[1] * unit[0]).abs();
            if off <= ON_RUNWAY
                && (-ON_RUNWAY..=length + ON_RUNWAY).contains(&along)
                && let Ok(id) = Id::from_usize(index)
            {
                nodes.push(id);
            }
        }
        nodes
    }

    /// How far `node` is from the nearest edge other than `own`, m.
    fn clearance(&self, node: Id<Node>, own: &[Id<Edge>]) -> f64 {
        let p = self.nodes()[node.as_usize()].xy;
        let mut nearest = f64::INFINITY;
        for (index, edge) in self.edges().iter().enumerate() {
            if Id::from_usize(index).is_ok_and(|id| own.contains(&id)) {
                continue;
            }
            for pair in edge.points().windows(2) {
                let [a, b] = [self.local(pair[0]), self.local(pair[1])];
                nearest = nearest.min(segment_distance(p, a, b));
            }
        }
        nearest
    }

    /// Dead ends of taxiways and taxi lanes that are neither stands nor on a
    /// runway (`on_runway` by node index): where an arrival can wait for a
    /// stand without blocking traffic.
    pub fn waiting_spots(&self, on_runway: &[bool]) -> Vec<Id<Node>> {
        let stands: Vec<Id<Node>> = self.stands().iter().map(super::Stand::node).collect();
        let mut spots = Vec::new();
        for index in 0..self.nodes().len() {
            let Ok(id) = Id::<Node>::from_usize(index) else {
                continue;
            };
            let edges = self.edges_at(id);
            let dead_end = edges.len() == 1
                && edges.iter().filter_map(|&e| self.edge(e)).all(|edge| {
                    matches!(edge.kind(), EdgeKind::Taxiway | EdgeKind::Taxilane)
                        && (!edge.oneway() || edge.to() == id)
                });
            if dead_end
                && !stands.contains(&id)
                && !on_runway.get(index).copied().unwrap_or(false)
                && self.clearance(id, edges) >= SPOT_CLEARANCE
            {
                spots.push(id);
            }
        }
        spots
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::{AirportGround, Node, SPOT_CLEARANCE};
    use crate::{Id, ground::GroundNetwork, test_data::data};

    /// Every spot keeps `SPOT_CLEARANCE` from the edges it is not on,
    /// sampled every 2 m, and leads on to a stand.
    fn assert_spots_clear(ground: &AirportGround, spots: &[Id<Node>]) -> Result<()> {
        let span = crate::units::Meters::new(1.0)?;
        for &spot in spots {
            assert!(
                ground
                    .stands()
                    .iter()
                    .any(|stand| ground.route(spot, stand.node(), span).is_some()),
                "no stand reachable from spot {}",
                spot.as_usize()
            );
            let at = ground.nodes()[spot.as_usize()].position();
            for (index, edge) in ground.edges().iter().enumerate() {
                if ground.edges_at(spot).contains(&Id::from_usize(index)?) {
                    continue;
                }
                for pair in edge.points().windows(2) {
                    // SAFETY: edge segments are short; a few hundred samples.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let steps = (pair[0].distance_to(pair[1]).value() / 2.0).ceil().max(1.0) as u32;
                    for step in 0..=steps {
                        let point =
                            pair[0].interpolate(pair[1], f64::from(step) / f64::from(steps))?;
                        let gap = point.distance_to(at).value();
                        assert!(
                            gap >= SPOT_CLEARANCE - 0.5,
                            "spot {} is {gap:.0} m from edge {index}",
                            spot.as_usize()
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_waiting_spots_at_munich_leave_the_taxiways_clear() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let munich = world.airport_by_ident("EDDM".parse()?).context("EDDM")?;
        let ground = network.airport(munich).context("layout")?;
        let mut on_runway = vec![false; ground.nodes().len()];
        for runway in world.runways_of(munich) {
            for node in ground.nodes_on(runway) {
                on_runway[node.as_usize()] = true;
            }
        }
        let spots = ground.waiting_spots(&on_runway);
        assert_ne!(spots.len(), 0);
        assert_spots_clear(ground, &spots)
    }

    #[test]
    fn test_runway_nodes_and_waiting_spots_at_frankfurt() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let ground = network.airport(frankfurt).context("layout")?;
        let mut on_runway = vec![false; ground.nodes().len()];
        for runway in world.runways_of(frankfurt) {
            let nodes = ground.nodes_on(runway);
            assert!(nodes.len() >= 5, "{}: {}", runway.name(), nodes.len());
            for node in nodes {
                on_runway[node.as_usize()] = true;
            }
        }
        // Every exit is on its runway.
        for exit in ground.exits() {
            assert!(on_runway[exit.node().as_usize()]);
        }
        let spots = ground.waiting_spots(&on_runway);
        assert_ne!(spots.len(), 0);
        assert_spots_clear(ground, &spots)?;
        for spot in &spots {
            assert!(!on_runway[spot.as_usize()]);
            assert_eq!(ground.edges_at(*spot).len(), 1);
            assert!(ground.stands().iter().all(|s| s.node() != *spot));
        }
        Ok(())
    }
}
