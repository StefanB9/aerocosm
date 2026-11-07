//! Shortest taxi routes on an airport's network (plan M4 D70 prerequisite):
//! A* by length, respecting one-way edges and wingspan limits, avoiding
//! runways and other stands' lead-ins where a taxiway will do.

use std::{cmp::Ordering, collections::BinaryHeap};

use super::{AirportGround, Edge, EdgeKind, Node};
use crate::{Id, LatLon, units::Meters};

/// Taxiing along a runway costs this many times its length: used only to
/// backtrack where no parallel taxiway exists.
const RUNWAY_PENALTY: f64 = 4.0;
/// A lead-in not at either end of the route costs this many times its
/// length, so routes do not cut through stands.
const LEAD_IN_PENALTY: f64 = 20.0;

/// A taxi route: the edges in order with their direction, and the polyline.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundRoute {
    edges: Vec<(Id<Edge>, bool)>,
    points: Vec<LatLon>,
    length: Meters,
}

impl GroundRoute {
    /// The edges taxied, each with `true` when taxied from its `from` node.
    pub fn edges(&self) -> &[(Id<Edge>, bool)] {
        &self.edges
    }

    /// The polyline from the start node to the end node.
    pub fn points(&self) -> &[LatLon] {
        &self.points
    }

    /// Length along the polyline.
    pub const fn length(&self) -> Meters {
        self.length
    }
}

/// A queue entry: estimated total cost, then node index for determinism.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry {
    estimate: f64,
    node: u32,
}

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: the heap pops the smallest estimate first.
        other
            .estimate
            .total_cmp(&self.estimate)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl AirportGround {
    /// Per node, whether a route from `from` leads there, respecting one-way
    /// edges (any wingspan).
    pub(crate) fn reachable_from(&self, from: Id<Node>) -> Vec<bool> {
        let mut seen = vec![false; self.nodes().len()];
        let Some(first) = seen.get_mut(from.as_usize()) else {
            return seen;
        };
        *first = true;
        let mut stack = vec![from];
        while let Some(here) = stack.pop() {
            for &edge in self.edges_at(here) {
                let Some(edge) = self.edge(edge) else {
                    continue;
                };
                let forward = edge.from() == here;
                if !forward && edge.oneway() {
                    continue;
                }
                let next = if forward { edge.to() } else { edge.from() };
                if let Some(slot) = seen.get_mut(next.as_usize())
                    && !*slot
                {
                    *slot = true;
                    stack.push(next);
                }
            }
        }
        seen
    }

    /// The cheapest route from `from` to `to` for an aircraft of `wingspan`,
    /// or `None` if there is none.
    pub fn route(&self, from: Id<Node>, to: Id<Node>, wingspan: Meters) -> Option<GroundRoute> {
        let count = self.nodes().len();
        let (start, goal) = (from.as_usize(), to.as_usize());
        if start >= count || goal >= count {
            return None;
        }
        let mut cost = vec![f64::INFINITY; count];
        let mut came: Vec<Option<(Id<Edge>, bool)>> = vec![None; count];
        let mut heap = BinaryHeap::with_capacity(64);
        cost[start] = 0.0;
        heap.push(Entry {
            estimate: self.remaining(start, goal),
            node: from.index(),
        });
        while let Some(Entry { estimate, node }) = heap.pop() {
            let here = node as usize;
            if here == goal {
                break;
            }
            if estimate - self.remaining(here, goal) > cost[here] + 1e-9 {
                // A stale entry: the node was reached more cheaply since.
                continue;
            }
            for &edge_id in self.edges_at(Id::new(node)) {
                let Some(edge) = self.edge(edge_id) else {
                    continue;
                };
                let forward = edge.from().as_usize() == here;
                if !forward && edge.oneway() {
                    continue;
                }
                if edge.max_wingspan().is_some_and(|limit| limit < wingspan) {
                    continue;
                }
                let next = if forward { edge.to() } else { edge.from() }.as_usize();
                let step = edge.length().value() * Self::penalty(edge, start, goal);
                let reached = cost[here] + step;
                if reached < cost[next] {
                    cost[next] = reached;
                    came[next] = Some((edge_id, forward));
                    heap.push(Entry {
                        estimate: reached + self.remaining(next, goal),
                        node: u32::try_from(next).ok()?,
                    });
                }
            }
        }
        if cost[goal].is_infinite() {
            return None;
        }
        self.trace(start, goal, &came)
    }

    /// How much more an edge costs than its length.
    fn penalty(edge: &Edge, start: usize, goal: usize) -> f64 {
        match edge.kind() {
            EdgeKind::Runway => RUNWAY_PENALTY,
            EdgeKind::LeadIn => {
                let ends = [edge.from().as_usize(), edge.to().as_usize()];
                if ends.contains(&start) || ends.contains(&goal) {
                    1.0
                } else {
                    LEAD_IN_PENALTY
                }
            }
            EdgeKind::Taxiway | EdgeKind::Taxilane => 1.0,
        }
    }

    /// Straight-line distance between two nodes: never more than a route.
    pub(crate) fn remaining(&self, from: usize, to: usize) -> f64 {
        let (a, b) = (self.nodes()[from].xy, self.nodes()[to].xy);
        (b[0] - a[0]).hypot(b[1] - a[1]) * 0.999
    }

    /// Walks the predecessor links back from `goal` and builds the route.
    fn trace(
        &self,
        start: usize,
        goal: usize,
        came: &[Option<(Id<Edge>, bool)>],
    ) -> Option<GroundRoute> {
        let mut edges = Vec::new();
        let mut node = goal;
        while node != start {
            let (edge_id, forward) = came[node]?;
            edges.push((edge_id, forward));
            let edge = self.edge(edge_id)?;
            node = if forward { edge.from() } else { edge.to() }.as_usize();
        }
        edges.reverse();
        let mut points = Vec::with_capacity(edges.len() * 4 + 1);
        points.push(self.nodes()[start].position());
        let mut length = 0.0;
        for &(edge_id, forward) in &edges {
            let edge = self.edge(edge_id)?;
            let line = edge.points();
            if forward {
                points.extend_from_slice(&line[1..]);
            } else {
                points.extend(line.iter().rev().skip(1));
            }
            length += edge.length().value();
        }
        Some(GroundRoute {
            edges,
            points,
            length: Meters::from_raw(length),
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        ground::{AirportGroundDefinition, EdgeDefinition, GroundNetwork, definition::Point},
        test_data::data,
        world::World,
    };

    /// Point `(x, y)` on a ~100 m grid near Frankfurt.
    fn at(x: f64, y: f64) -> Point {
        [50.03 + y * 0.0009, 8.56 + x * 0.0014]
    }

    fn edge(from: u32, to: u32, kind: EdgeKind) -> EdgeDefinition {
        EdgeDefinition {
            from,
            to,
            kind,
            name: None,
            oneway: false,
            max_wingspan: None,
            points: Vec::new(),
        }
    }

    fn layout(nodes: Vec<Point>, edges: Vec<EdgeDefinition>) -> Result<(World, AirportGround)> {
        let world = data()?.world;
        let ground = AirportGround::new(
            AirportGroundDefinition {
                airport: "EDDF".parse()?,
                nodes,
                edges,
                stands: Vec::new(),
                holding_positions: Vec::new(),
                exits: Vec::new(),
                aprons: Vec::new(),
            },
            &world,
        )?;
        Ok((world, ground))
    }

    fn span(metres: f64) -> Result<Meters> {
        Meters::new(metres)
    }

    fn nodes_of(ground: &AirportGround, route: &GroundRoute) -> Vec<u32> {
        let mut nodes = Vec::new();
        for &(edge, forward) in route.edges() {
            if let Some(edge) = ground.edge(edge) {
                let (a, b) = if forward {
                    (edge.from(), edge.to())
                } else {
                    (edge.to(), edge.from())
                };
                if nodes.is_empty() {
                    nodes.push(a.index());
                }
                nodes.push(b.index());
            }
        }
        nodes
    }

    /// A square 0-1-2-3 with a long way round 0-4-5-2.
    fn square() -> Result<(World, AirportGround)> {
        layout(
            vec![
                at(0.0, 0.0),
                at(1.0, 0.0),
                at(1.0, 1.0),
                at(0.0, 1.0),
                at(-1.0, 0.0),
                at(-1.0, 2.0),
            ],
            vec![
                edge(0, 1, EdgeKind::Taxiway),
                edge(1, 2, EdgeKind::Taxiway),
                edge(2, 3, EdgeKind::Taxiway),
                edge(3, 0, EdgeKind::Taxiway),
                edge(0, 4, EdgeKind::Taxiway),
                edge(4, 5, EdgeKind::Taxiway),
                edge(5, 2, EdgeKind::Taxiway),
            ],
        )
    }

    #[test]
    fn test_route_takes_the_shortest_way() -> Result<()> {
        let (_, ground) = square()?;
        let route = ground
            .route(Id::new(0), Id::new(2), span(36.0)?)
            .context("a route")?;
        assert_eq!(route.edges().len(), 2);
        assert!(
            (route.length().value() - 200.0).abs() < 2.0,
            "{:?}",
            route.length()
        );
        assert_eq!(route.points().len(), 3);
        let first = route.points()[0];
        assert!(first.distance_to(ground.nodes()[0].position()).value() < 0.01);
        let last = route.points()[2];
        assert!(last.distance_to(ground.nodes()[2].position()).value() < 0.01);
        // Backwards along edge 3-0 and 2-3 is just as short; either way the
        // route walks edges in the order it travels them.
        let back = ground
            .route(Id::new(2), Id::new(0), span(36.0)?)
            .context("a route back")?;
        assert_eq!(nodes_of(&ground, &back).first(), Some(&2));
        assert_eq!(nodes_of(&ground, &back).last(), Some(&0));
        let same = ground
            .route(Id::new(1), Id::new(1), span(36.0)?)
            .context("a trivial route")?;
        assert_eq!(same.edges().len(), 0);
        assert_eq!(same.points().len(), 1);
        assert!(ground.route(Id::new(0), Id::new(99), span(36.0)?).is_none());
        Ok(())
    }

    #[test]
    fn test_route_respects_one_way_edges_and_wingspans() -> Result<()> {
        let (_, mut ground) = square()?;
        // One-way 1→0 and 2→3 forces 0→2 the long way round.
        ground.edges[0] = Edge {
            from: Id::new(1),
            to: Id::new(0),
            oneway: true,
            ..ground.edges[0].clone()
        };
        ground.edges[2].oneway = true;
        let route = ground
            .route(Id::new(0), Id::new(2), span(36.0)?)
            .context("the long way")?;
        assert_eq!(nodes_of(&ground, &route), vec![0, 4, 5, 2]);
        // The long way is closed to wide aircraft: no route at all.
        ground.edges[5].max_wingspan = Some(span(52.0)?);
        assert!(ground.route(Id::new(0), Id::new(2), span(65.0)?).is_none());
        assert!(ground.route(Id::new(0), Id::new(2), span(36.0)?).is_some());
        Ok(())
    }

    #[test]
    fn test_reachable_from_follows_one_way_edges() -> Result<()> {
        let (_, mut ground) = square()?;
        assert!(ground.reachable_from(Id::new(0)).iter().all(|&r| r));
        // A one-way spur 0→4 with no way back: from 4 only 5, 2, 3, 1 and 0
        // via the square, but nothing reaches 4 from 5.
        ground.edges[4].oneway = true;
        ground.edges[5].oneway = true;
        ground.edges[5] = Edge {
            from: Id::new(5),
            to: Id::new(4),
            ..ground.edges[5].clone()
        };
        let from_four = ground.reachable_from(Id::new(4));
        assert_eq!(from_four, vec![false, false, false, false, true, false]);
        assert!(ground.reachable_from(Id::new(5))[4]);
        assert_eq!(ground.reachable_from(Id::new(99)), vec![false; 6]);
        Ok(())
    }

    #[test]
    fn test_route_avoids_runways_and_other_stands() -> Result<()> {
        // Straight across 0-1 is a runway, the way round 0-2-3-1 is ~2.4x as
        // long. A lead-in 4-5 shortcuts 3-1.
        let (_, ground) = layout(
            vec![
                at(0.0, 0.0),
                at(3.0, 0.0),
                at(0.0, 1.0),
                at(3.0, 2.0),
                at(3.0, 1.0),
                at(2.0, 1.0),
            ],
            vec![
                edge(0, 1, EdgeKind::Runway),
                edge(0, 2, EdgeKind::Taxiway),
                edge(2, 5, EdgeKind::Taxiway),
                edge(5, 3, EdgeKind::Taxiway),
                edge(3, 1, EdgeKind::Taxiway),
                edge(5, 4, EdgeKind::LeadIn),
                edge(4, 1, EdgeKind::LeadIn),
            ],
        )?;
        let route = ground
            .route(Id::new(0), Id::new(1), span(36.0)?)
            .context("a route")?;
        assert_eq!(nodes_of(&ground, &route), vec![0, 2, 5, 3, 1]);
        // To the stand at 4 the lead-in is the way in.
        let to_stand = ground
            .route(Id::new(0), Id::new(4), span(36.0)?)
            .context("to the stand")?;
        assert_eq!(nodes_of(&ground, &to_stand), vec![0, 2, 5, 4]);
        Ok(())
    }

    fn shipped() -> Result<(World, GroundNetwork)> {
        let world = data()?.world;
        let ground = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        Ok((world, ground))
    }

    static SHIPPED: std::sync::LazyLock<Result<(World, GroundNetwork), String>> =
        std::sync::LazyLock::new(|| shipped().map_err(|e| format!("{e:#}")));

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_routes_on_shipped_layouts_are_continuous(
            airport in 0_usize..22, stand in 0_usize..400, hold in 0_usize..400,
        ) {
            let (_, network) = SHIPPED.as_ref().map_err(|e| TestCaseError::fail(e.clone()))?;
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let ground = &network.airports()[airport % network.airports().len()];
            let stand = &ground.stands()[stand % ground.stands().len()];
            let holding = &ground.holding_positions()[hold % ground.holding_positions().len()];
            let Some(route) = ground.route(stand.node(), holding.node(), span(36.0).map_err(fail)?)
            else {
                // Some stands only reach some runways through one-way edges.
                return Ok(());
            };
            let points = route.points();
            let start = ground.nodes()[stand.node().as_usize()].position();
            prop_assert!(points[0].distance_to(start).value() < 0.01);
            let end = ground.nodes()[holding.node().as_usize()].position();
            prop_assert!(points[points.len() - 1].distance_to(end).value() < 0.01);
            let walked: f64 = points.windows(2).map(|w| w[0].distance_to(w[1]).value()).sum();
            prop_assert!((walked - route.length().value()).abs() < 1.0);
            prop_assert!(route.length().value() + 1.0 >= points[0].distance_to(end).value());
            for &(edge, forward) in route.edges() {
                let edge = ground.edge(edge).ok_or_else(|| TestCaseError::fail("edge"))?;
                prop_assert!(forward || !edge.oneway());
            }
        }
    }

    #[test]
    fn test_most_stands_reach_a_runway() -> Result<()> {
        let (_, network) = shipped()?;
        let ground = network.airports().first().context("a layout")?;
        let holding = ground.holding_positions().first().context("a hold")?;
        let reached = ground
            .stands()
            .iter()
            .filter(|stand| {
                span(36.0)
                    .ok()
                    .and_then(|s| ground.route(stand.node(), holding.node(), s))
                    .is_some()
            })
            .count();
        assert!(reached * 10 >= ground.stands().len() * 9, "{reached}");
        Ok(())
    }
}
