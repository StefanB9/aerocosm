//! OSM ways → a taxi graph split at junctions (plan M4 D67), in a local
//! metric frame.

use std::collections::{HashMap, HashSet};

use aerocosm_core::{
    ground::{EdgeKind, GroundName},
    units::Meters,
};

use super::{
    local::{Plane, Xy, closest_on_segment, distance},
    names::ground_name,
    overpass::{OsmData, OsmWay},
};
use crate::geometry::simplify;

/// Interior points closer than this to the simplified line are dropped.
const SIMPLIFY_TOLERANCE: f64 = 0.5;

/// A graph node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GraphNode {
    pub(crate) xy: Xy,
    /// The OSM node it came from, if any.
    pub(crate) osm: Option<i64>,
}

/// A graph edge with its interior points.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GraphEdge {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) kind: EdgeKind,
    pub(crate) name: Option<GroundName>,
    pub(crate) oneway: bool,
    pub(crate) max_wingspan: Option<Meters>,
    pub(crate) points: Vec<Xy>,
    /// The OSM way it came from; `None` for connectors.
    pub(crate) way: Option<i64>,
}

impl GraphEdge {
    /// The whole polyline, both nodes included.
    pub(crate) fn polyline(&self, nodes: &[GraphNode]) -> Vec<Xy> {
        let mut line = Vec::with_capacity(self.points.len() + 2);
        line.push(nodes[self.from].xy);
        line.extend_from_slice(&self.points);
        line.push(nodes[self.to].xy);
        line
    }

    /// The first point after `node` when leaving through this edge.
    pub(crate) fn next_after(&self, node: usize, nodes: &[GraphNode]) -> Xy {
        if node == self.from {
            self.points.first().copied().unwrap_or(nodes[self.to].xy)
        } else {
            self.points.last().copied().unwrap_or(nodes[self.from].xy)
        }
    }

    /// The node at the other end from `node`.
    pub(crate) const fn other(&self, node: usize) -> usize {
        if node == self.from {
            self.to
        } else {
            self.from
        }
    }
}

/// The taxi network of one airport.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Graph {
    pub(crate) nodes: Vec<GraphNode>,
    pub(crate) edges: Vec<GraphEdge>,
    by_osm: HashMap<i64, usize>,
}

/// The ground kind of a way, if it belongs to the network.
pub(crate) fn way_kind(way: &OsmWay) -> Option<EdgeKind> {
    if way.is_area() || way.nodes.len() < 2 {
        return None;
    }
    match way.aeroway()? {
        "taxiway" => Some(EdgeKind::Taxiway),
        "taxilane" => Some(EdgeKind::Taxilane),
        "runway" => Some(EdgeKind::Runway),
        "parking_position" => Some(EdgeKind::LeadIn),
        _ => None,
    }
}

/// `oneway` as (one-way, reversed).
fn oneway(way: &OsmWay) -> (bool, bool) {
    match way.tags.get("oneway").map(String::as_str) {
        Some("yes" | "1" | "true") => (true, false),
        Some("-1") => (true, true),
        _ => (false, false),
    }
}

/// A wingspan from a `maxspan` tag such as `36`, `36 m` or `65.0`.
fn max_wingspan(way: &OsmWay) -> Option<Meters> {
    let text = way.tags.get("maxspan")?;
    let number = text.trim().trim_end_matches('m').trim();
    let value: f64 = number.parse().ok()?;
    (value.is_finite() && value > 0.0)
        .then(|| Meters::new(value).ok())
        .flatten()
}

impl Graph {
    /// Builds the graph from the network ways selected by `keep`, splitting
    /// them at nodes shared with other ways, at their ends and at the extra
    /// `junctions` (e.g. holding positions).
    pub(crate) fn build(
        data: &OsmData,
        plane: &Plane,
        keep: impl Fn(&OsmWay) -> bool,
        junctions: &HashSet<i64>,
    ) -> Self {
        let ways: Vec<(&OsmWay, EdgeKind)> = data
            .ways
            .iter()
            .filter_map(|way| way_kind(way).map(|kind| (way, kind)))
            .filter(|(way, _)| way.nodes.iter().all(|id| data.nodes.contains_key(id)))
            .filter(|(way, _)| keep(way))
            .collect();
        let mut uses: HashMap<i64, u32> = HashMap::new();
        for (way, _) in &ways {
            for id in &way.nodes {
                *uses.entry(*id).or_default() += 1;
            }
        }
        let mut graph = Self::default();
        for (way, kind) in ways {
            graph.add_way(data, plane, way, kind, &uses, junctions);
        }
        graph
    }

    fn node_for(&mut self, id: i64, xy: Xy) -> usize {
        if let Some(&index) = self.by_osm.get(&id) {
            return index;
        }
        let index = self.nodes.len();
        self.nodes.push(GraphNode { xy, osm: Some(id) });
        self.by_osm.insert(id, index);
        index
    }

    /// The graph node of OSM node `id`, if it is one.
    pub(crate) fn node_of(&self, id: i64) -> Option<usize> {
        self.by_osm.get(&id).copied()
    }

    fn add_way(
        &mut self,
        data: &OsmData,
        plane: &Plane,
        way: &OsmWay,
        kind: EdgeKind,
        uses: &HashMap<i64, u32>,
        junctions: &HashSet<i64>,
    ) {
        let (oneway, reversed) = oneway(way);
        let name = way
            .tags
            .get("ref")
            .or_else(|| way.tags.get("name"))
            .and_then(|text| ground_name(text));
        let span = max_wingspan(way);
        let last = way.nodes.len() - 1;
        let xy = |id: i64| data.position(id).map_or([0.0; 2], |p| plane.project(p));
        let mut start = way.nodes[0];
        let mut interior: Vec<(i64, Xy)> = Vec::new();
        for (index, &id) in way.nodes.iter().enumerate().skip(1) {
            let split =
                index == last || uses.get(&id).copied().unwrap_or(0) > 1 || junctions.contains(&id);
            if !split {
                interior.push((id, xy(id)));
                continue;
            }
            let mut points = std::mem::take(&mut interior);
            let pieces = if start != id {
                vec![(start, points, id)]
            } else if points.is_empty() {
                // A repeated node: nothing to add.
                Vec::new()
            } else {
                // A loop back to its start is split at its middle point.
                let middle = points.len() / 2;
                let after = points.split_off(middle + 1);
                let (mid_id, _) = points.pop().unwrap_or((id, [0.0; 2]));
                vec![(start, points, mid_id), (mid_id, after, id)]
            };
            for (from, points, to) in pieces {
                let from_index = self.node_for(from, xy(from));
                let to_index = self.node_for(to, xy(to));
                let mut line: Vec<Xy> = Vec::with_capacity(points.len() + 2);
                line.push(self.nodes[from_index].xy);
                line.extend(points.iter().map(|(_, p)| *p));
                line.push(self.nodes[to_index].xy);
                let mut kept = simplify(&line, SIMPLIFY_TOLERANCE);
                kept.pop();
                kept.remove(0);
                let (from_index, to_index, kept) = if reversed {
                    kept.reverse();
                    (to_index, from_index, kept)
                } else {
                    (from_index, to_index, kept)
                };
                self.edges.push(GraphEdge {
                    from: from_index,
                    to: to_index,
                    kind,
                    name,
                    oneway,
                    max_wingspan: span,
                    points: kept,
                    way: Some(way.id),
                });
            }
            start = id;
        }
    }

    /// The edges touching each node.
    pub(crate) fn adjacency(&self) -> Vec<Vec<usize>> {
        let mut adjacency = vec![Vec::new(); self.nodes.len()];
        for (index, edge) in self.edges.iter().enumerate() {
            adjacency[edge.from].push(index);
            adjacency[edge.to].push(index);
        }
        adjacency
    }

    /// The point of an edge accepted by `filter` nearest to `p`, within
    /// `limit`: (edge, segment index, point, distance).
    pub(crate) fn nearest_on_edges(
        &self,
        p: Xy,
        limit: f64,
        filter: impl Fn(usize, &GraphEdge) -> bool,
    ) -> Option<(usize, usize, Xy, f64)> {
        let mut best: Option<(usize, usize, Xy, f64)> = None;
        for (index, edge) in self.edges.iter().enumerate() {
            if !filter(index, edge) {
                continue;
            }
            let line = edge.polyline(&self.nodes);
            for (segment, pair) in line.windows(2).enumerate() {
                let (q, _) = closest_on_segment(p, pair[0], pair[1]);
                let d = distance(p, q);
                if d <= limit && best.is_none_or(|(_, _, _, least)| d < least) {
                    best = Some((index, segment, q, d));
                }
            }
        }
        best
    }

    /// Splits `edge` at `point` on its segment `segment`, returning the new
    /// node; the second half keeps the edge's attributes.
    pub(crate) fn split_edge(&mut self, edge: usize, segment: usize, point: Xy) -> usize {
        let line = self.edges[edge].polyline(&self.nodes);
        // Reuse an end node when the point is on it.
        if distance(point, line[0]) < 0.01 {
            return self.edges[edge].from;
        }
        if distance(point, line[line.len() - 1]) < 0.01 {
            return self.edges[edge].to;
        }
        let node = self.nodes.len();
        self.nodes.push(GraphNode {
            xy: point,
            osm: None,
        });
        let original = self.edges[edge].clone();
        // Interior points before the split are `line[1..=segment]`.
        let before: Vec<Xy> = line[1..=segment].to_vec();
        let after: Vec<Xy> = line[segment + 1..line.len() - 1].to_vec();
        self.edges[edge] = GraphEdge {
            to: node,
            points: before,
            ..original.clone()
        };
        self.edges.push(GraphEdge {
            from: node,
            points: after,
            ..original
        });
        node
    }

    /// Adds a straight connector edge.
    pub(crate) fn connect(&mut self, from: usize, to: usize, kind: EdgeKind) {
        self.edges.push(GraphEdge {
            from,
            to,
            kind,
            name: None,
            oneway: false,
            max_wingspan: None,
            points: Vec::new(),
            way: None,
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashSet;

    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;
    use crate::osm::overpass::read_overpass;

    pub(crate) const PLANE_ORIGIN: [f64; 2] = [50.0, 8.5];

    /// Nodes on a grid of ~10 m steps: node `i` at row `i / 100`, column
    /// `i % 100`.
    pub(crate) fn grid_node(id: i64) -> String {
        let row = f64::from(i32::try_from(id / 100).unwrap_or(0));
        let column = f64::from(i32::try_from(id % 100).unwrap_or(0));
        let lat = 50.0 + row * 0.0001;
        let lon = 8.5 + column * 0.00014;
        format!("{{\"type\":\"node\",\"id\":{id},\"lat\":{lat},\"lon\":{lon}}}")
    }

    pub(crate) fn way(id: i64, nodes: &[i64], tags: &str) -> String {
        let list: Vec<String> = nodes.iter().map(i64::to_string).collect();
        format!(
            "{{\"type\":\"way\",\"id\":{id},\"nodes\":[{}],\"tags\":{{{tags}}}}}",
            list.join(",")
        )
    }

    pub(crate) fn data(ways: &[String], extra_nodes: &[String]) -> Result<OsmData> {
        let mut ids: Vec<i64> = Vec::new();
        for way in ways {
            let start = way.find('[').unwrap_or(0) + 1;
            let end = way.find(']').unwrap_or(start);
            ids.extend(
                way[start..end]
                    .split(',')
                    .filter_map(|s| s.parse::<i64>().ok()),
            );
        }
        ids.sort_unstable();
        ids.dedup();
        let mut elements: Vec<String> = ids.into_iter().map(grid_node).collect();
        elements.extend_from_slice(ways);
        elements.extend_from_slice(extra_nodes);
        read_overpass(&format!("{{\"elements\":[{}]}}", elements.join(",")))
    }

    fn build(data: &OsmData) -> Graph {
        Graph::build(data, &Plane::new(PLANE_ORIGIN), |_| true, &HashSet::new())
    }

    #[test]
    fn test_graph_splits_ways_at_shared_nodes() -> Result<()> {
        // A taxiway along row 0 crossed by a taxilane along column 2.
        let data = data(
            &[
                way(1, &[0, 1, 2, 3, 4], "\"aeroway\":\"taxiway\",\"ref\":\"N\""),
                way(2, &[202, 102, 2], "\"aeroway\":\"taxilane\""),
                way(3, &[300, 301], "\"aeroway\":\"apron\""),
            ],
            &[],
        )?;
        let graph = build(&data);
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.nodes.len(), 4);
        let first = &graph.edges[0];
        assert_eq!(first.kind, EdgeKind::Taxiway);
        assert_eq!(first.name.map(|n| n.to_string()), Some("N".to_owned()));
        // Straight interior points are simplified away.
        assert_eq!(first.points.len(), 0);
        let junction = graph.node_of(2).ok_or_else(|| anyhow::anyhow!("node 2"))?;
        assert_eq!(graph.adjacency()[junction].len(), 3);
        assert_eq!(graph.node_of(300), None);
        Ok(())
    }

    #[test]
    fn test_graph_handles_oneway_loops_and_extra_junctions() -> Result<()> {
        let data = data(
            &[
                way(1, &[0, 1, 2], "\"aeroway\":\"taxiway\",\"oneway\":\"-1\""),
                way(2, &[10, 11, 111, 110, 10], "\"aeroway\":\"taxilane\""),
                way(
                    3,
                    &[20, 21, 22],
                    "\"aeroway\":\"taxiway\",\"maxspan\":\"36 m\"",
                ),
            ],
            &[],
        )?;
        let junctions: HashSet<i64> = [21].into();
        let graph = Graph::build(&data, &Plane::new(PLANE_ORIGIN), |_| true, &junctions);
        let reversed = &graph.edges[0];
        assert!(reversed.oneway);
        assert_eq!(graph.nodes[reversed.from].osm, Some(2));
        assert_eq!(graph.nodes[reversed.to].osm, Some(0));
        // The closed lane becomes two edges, never one from a node to itself.
        let lane: Vec<&GraphEdge> = graph.edges.iter().filter(|e| e.way == Some(2)).collect();
        assert_eq!(lane.len(), 2);
        assert!(lane.iter().all(|e| e.from != e.to));
        let split: Vec<&GraphEdge> = graph.edges.iter().filter(|e| e.way == Some(3)).collect();
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].max_wingspan, Some(Meters::new(36.0)?));
        Ok(())
    }

    #[test]
    fn test_graph_splits_edges_and_connects() -> Result<()> {
        let data = data(&[way(1, &[0, 1, 2, 3, 4], "\"aeroway\":\"taxiway\"")], &[])?;
        let mut graph = build(&data);
        let p = graph.nodes[0].xy;
        let q = graph.nodes[1].xy;
        let target = [f64::midpoint(p[0], q[0]), p[1] + 20.0];
        let (edge, segment, point, d) = graph
            .nearest_on_edges(target, 50.0, |_, _| true)
            .ok_or_else(|| anyhow::anyhow!("an edge is near"))?;
        assert!((d - 20.0).abs() < 1e-6);
        let node = graph.split_edge(edge, segment, point);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].to, node);
        assert_eq!(graph.edges[1].from, node);
        let total: f64 = graph
            .edges
            .iter()
            .map(|e| {
                e.polyline(&graph.nodes)
                    .windows(2)
                    .map(|w| distance(w[0], w[1]))
                    .sum::<f64>()
            })
            .sum();
        assert!((total - distance(p, q)).abs() < 1e-6);
        // Splitting at an end reuses its node.
        assert_eq!(graph.split_edge(0, 0, p), graph.edges[0].from);
        assert!(
            graph
                .nearest_on_edges([0.0, 5_000.0], 50.0, |_, _| true)
                .is_none()
        );
        let stand = graph.nodes.len();
        graph.nodes.push(GraphNode {
            xy: target,
            osm: None,
        });
        graph.connect(node, stand, EdgeKind::LeadIn);
        assert_eq!(graph.edges[2].kind, EdgeKind::LeadIn);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_graph_keeps_every_way_connected(
            ways in prop::collection::vec(prop::collection::vec(0_i64..12, 2..6), 1..6),
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let encoded: Vec<String> = ways
                .iter()
                .zip(1_i64..)
                .map(|(nodes, id)| way(id, nodes, "\"aeroway\":\"taxiway\""))
                .collect();
            let data = data(&encoded, &[]).map_err(fail)?;
            let graph = build(&data);
            let adjacency = graph.adjacency();
            for edge in &graph.edges {
                prop_assert!(edge.from != edge.to);
            }
            // Consecutive OSM nodes of a way that are both graph nodes stay
            // connected through the way's edges.
            for (nodes, way_id) in ways.iter().zip(1_i64..) {
                let mut ends: Vec<usize> = graph
                    .edges
                    .iter()
                    .filter(|e| e.way == Some(way_id))
                    .flat_map(|e| [e.from, e.to])
                    .collect();
                ends.sort_unstable();
                ends.dedup();
                for id in [nodes[0], nodes[nodes.len() - 1]] {
                    if let Some(node) = graph.node_of(id)
                        && nodes.windows(2).any(|w| w[0] != w[1])
                    {
                        prop_assert!(ends.contains(&node) || adjacency[node].is_empty());
                    }
                }
            }
        }
    }
}
