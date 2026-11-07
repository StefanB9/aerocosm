//! Where the network meets runways (plan M4 D77) and where arrivals can wait
//! for a stand (D76).

use super::{AirportGround, EdgeKind, Node};
use crate::{Id, LatLon, world::Runway};

/// Nodes within this distance of a runway centreline are on the runway, m.
const ON_RUNWAY: f64 = 30.0;

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
            if dead_end && !stands.contains(&id) && !on_runway.get(index).copied().unwrap_or(false)
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

    use crate::{ground::GroundNetwork, test_data::data};

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
        for spot in &spots {
            assert!(!on_runway[spot.as_usize()]);
            assert_eq!(ground.edges_at(*spot).len(), 1);
            assert!(ground.stands().iter().all(|s| s.node() != *spot));
        }
        Ok(())
    }
}
