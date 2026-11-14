//! Holding positions and runway exits (plan M4 D67, D71), matched to the
//! world's runway ends.

use aerocosm_core::{
    ground::EdgeKind,
    world::{Runway, RunwayDesignator},
};

use super::{
    graph::{Graph, GraphEdge},
    local::{Plane, Xy, angle_between, bearing, distance},
    overpass::OsmData,
};

/// Nodes within this distance of a runway centreline are on the runway.
const ON_RUNWAY: f64 = 25.0;
/// Holding positions belong to a runway within this distance of its
/// centreline (they stand 90–150 m off it, more at some airports).
pub(crate) const HOLDING_REACH: f64 = 300.0;
/// Exits turning more than this off the landing direction are not used.
const MAX_EXIT_ANGLE: f64 = 120.0;
/// An exit's direction is taken this far along it, past the curve that
/// often starts on the runway.
const EXIT_PROBE: f64 = 60.0;
/// At the probe, an exit is at least this far off the centreline.
const LEAVES_RUNWAY: f64 = 5.0;

/// The point `EXIT_PROBE` along `edge` from `node` (or its far end).
fn ahead_of(edge: &GraphEdge, node: usize, graph: &Graph) -> Xy {
    let mut line = edge.polyline(&graph.nodes);
    if node != edge.from {
        line.reverse();
    }
    let mut left = EXIT_PROBE;
    for pair in line.windows(2) {
        let step = distance(pair[0], pair[1]);
        if step >= left && step > 0.0 {
            let t = left / step;
            return [
                pair[0][0] + t * (pair[1][0] - pair[0][0]),
                pair[0][1] + t * (pair[1][1] - pair[0][1]),
            ];
        }
        left -= step;
    }
    line.last().copied().unwrap_or_default()
}

/// A runway in the local frame: its two thresholds and designators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LocalRunway {
    pub(crate) ends: [(RunwayDesignator, Xy); 2],
}

impl LocalRunway {
    /// `runway` projected into `plane`.
    pub(crate) fn new(runway: &Runway, plane: &Plane) -> Self {
        let end = |i: usize| {
            let end = &runway.ends()[i];
            let at = end.threshold();
            (
                end.designator(),
                plane.project([at.latitude().value(), at.longitude().value()]),
            )
        };
        Self {
            ends: [end(0), end(1)],
        }
    }

    pub(crate) fn length(&self) -> f64 {
        distance(self.ends[0].1, self.ends[1].1)
    }

    /// (distance along from end `end`'s threshold, distance off the
    /// centreline) of `p`.
    pub(crate) fn locate(&self, point: Xy, end: usize) -> (f64, f64) {
        let from = self.ends[end].1;
        let to = self.ends[1 - end].1;
        let length = self.length().max(f64::EPSILON);
        let unit = [(to[0] - from[0]) / length, (to[1] - from[1]) / length];
        let offset = [point[0] - from[0], point[1] - from[1]];
        let along = offset[0] * unit[0] + offset[1] * unit[1];
        let off = (offset[0] * unit[1] - offset[1] * unit[0]).abs();
        (along, off)
    }

    /// Whether `p` lies on the runway's centreline between its thresholds.
    pub(crate) fn contains(&self, p: Xy) -> bool {
        let (along, off) = self.locate(p, 0);
        off <= ON_RUNWAY && (-ON_RUNWAY..=self.length() + ON_RUNWAY).contains(&along)
    }
}

/// A holding position found in the data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawHolding {
    pub(crate) node: usize,
    pub(crate) runway: RunwayDesignator,
}

/// Runway holding positions on the graph, each assigned to the runway end
/// whose threshold is nearer: departures from that end wait there.
pub(crate) fn holding_positions(
    graph: &Graph,
    data: &OsmData,
    runways: &[LocalRunway],
) -> Vec<RawHolding> {
    let mut nodes: Vec<_> = data
        .nodes
        .values()
        .filter(|node| {
            node.tags
                .get("aeroway")
                .is_some_and(|a| a == "holding_position")
        })
        .filter(|node| {
            node.tags
                .get("holding_position:type")
                .is_none_or(|kind| kind == "runway")
        })
        .filter_map(|node| graph.node_of(node.id))
        .collect();
    nodes.sort_unstable();
    let mut found = Vec::with_capacity(nodes.len());
    for node in nodes {
        let p = graph.nodes[node].xy;
        let best = runways
            .iter()
            .filter_map(|runway| {
                let length = runway.length();
                let (along, off) = runway.locate(p, 0);
                (off <= HOLDING_REACH && (-HOLDING_REACH..=length + HOLDING_REACH).contains(&along))
                    .then(|| {
                        let end = usize::from(along > length / 2.0);
                        (off, runway.ends[end].0)
                    })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, runway)) = best {
            found.push(RawHolding { node, runway });
        }
    }
    found
}

/// Whether `tags` mark an intermediate holding position.
fn intermediate(tags: &super::overpass::Tags) -> bool {
    tags.get("aeroway").is_some_and(|a| a == "holding_position")
        && tags
            .get("holding_position:type")
            .is_some_and(|kind| kind == "intermediate")
}

/// OSM nodes where an intermediate holding position drawn as a line across
/// a taxiway meets the taxiway (plan M4 D89): the graph is split there.
pub(crate) fn hold_lines(data: &OsmData) -> Vec<i64> {
    let taxiway: std::collections::HashSet<i64> = data
        .ways
        .iter()
        .filter(|way| matches!(way.aeroway(), Some("taxiway" | "taxilane")))
        .flat_map(|way| way.nodes.iter().copied())
        .collect();
    let mut nodes: Vec<i64> = data
        .ways
        .iter()
        .filter(|way| intermediate(&way.tags))
        .flat_map(|way| way.nodes.iter().copied())
        .filter(|id| taxiway.contains(id))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

/// Graph nodes with an intermediate holding position, tagged on the node
/// or where a hold line crosses the taxiway (plan M4 D89).
pub(crate) fn intermediate_holds(graph: &Graph, data: &OsmData) -> Vec<usize> {
    let mut nodes: Vec<usize> = data
        .nodes
        .values()
        .filter(|node| intermediate(&node.tags))
        .map(|node| node.id)
        .chain(hold_lines(data))
        .filter_map(|id| graph.node_of(id))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

/// A runway exit found in the data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawExit {
    pub(crate) runway: RunwayDesignator,
    pub(crate) node: usize,
    pub(crate) distance: f64,
    pub(crate) angle: f64,
}

/// For each runway end, the graph nodes on its centreline where a taxiway
/// leaves at most 120° off the landing direction (measured 60 m along it), by
/// distance from the threshold.
pub(crate) fn exits(graph: &Graph, runways: &[LocalRunway]) -> Vec<RawExit> {
    let adjacency = graph.adjacency();
    let mut found = Vec::new();
    for runway in runways {
        for end in 0..2 {
            let heading = bearing(runway.ends[end].1, runway.ends[1 - end].1);
            let mut on_end: Vec<RawExit> = Vec::new();
            for (node, links) in adjacency.iter().enumerate() {
                let p = graph.nodes[node].xy;
                if !runway.contains(p) {
                    continue;
                }
                let angle = links
                    .iter()
                    .map(|&edge| &graph.edges[edge])
                    .filter(|edge| matches!(edge.kind, EdgeKind::Taxiway | EdgeKind::Taxilane))
                    .filter_map(|edge| {
                        let ahead = ahead_of(edge, node, graph);
                        let (_, off) = runway.locate(ahead, 0);
                        (off > LEAVES_RUNWAY).then(|| angle_between(heading, bearing(p, ahead)))
                    })
                    .fold(f64::INFINITY, f64::min);
                if angle <= MAX_EXIT_ANGLE {
                    let (along, _) = runway.locate(p, end);
                    on_end.push(RawExit {
                        runway: runway.ends[end].0,
                        node,
                        distance: along.max(0.0),
                        angle,
                    });
                }
            }
            on_end.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.node.cmp(&b.node)));
            found.extend(on_end);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use anyhow::Result;

    use super::*;
    use crate::osm::graph::tests::{PLANE_ORIGIN, data, grid_node, way};

    /// A runway "09/27" along row 0 from column 0 to column 30 (≈ 300 m).
    fn runway(plane: &Plane) -> Result<LocalRunway> {
        Ok(LocalRunway {
            ends: [
                ("09".parse()?, plane.project([50.0, 8.5])),
                ("27".parse()?, plane.project([50.0, 8.5 + 30.0 * 0.00014])),
            ],
        })
    }

    #[test]
    fn test_local_runway_locates_points() -> Result<()> {
        let plane = Plane::new(PLANE_ORIGIN);
        let runway = runway(&plane)?;
        let p = plane.project([50.0001, 8.5 + 10.0 * 0.00014]);
        let (along, off) = runway.locate(p, 0);
        assert!(
            (along - 100.0).abs() < 1.0 && (off - 11.1).abs() < 0.2,
            "{along} {off}"
        );
        let (back, _) = runway.locate(p, 1);
        assert!((back - 200.0).abs() < 1.0);
        assert!(runway.contains(p));
        assert!(!runway.contains(plane.project([50.001, 8.5])));
        Ok(())
    }

    #[test]
    fn test_holding_positions_and_exits_attach_to_runway_ends() -> Result<()> {
        let holding = |id: i64, kind: &str| {
            let node = grid_node(id);
            format!(
                "{},\"tags\":{{\"aeroway\":\"holding_position\",\"holding_position:type\":\"\
                 {kind}\"}}}}",
                &node[..node.len() - 1]
            )
        };
        // Runway centreline on row 0; a perpendicular taxiway at column 2 with
        // a holding position at row 8; an angled exit from column 20 to the
        // north-west; a taxiway at column 28 with an intermediate holding
        // point.
        let data = data(
            &[
                way(
                    1,
                    &[0, 2, 20, 28, 30],
                    "\"aeroway\":\"runway\",\"ref\":\"09/27\"",
                ),
                way(2, &[2, 802, 1602], "\"aeroway\":\"taxiway\""),
                way(3, &[20, 118, 812], "\"aeroway\":\"taxiway\""),
                way(4, &[28, 828, 1628], "\"aeroway\":\"taxiway\""),
                // An intermediate hold drawn as a line across taxiway 3.
                way(
                    5,
                    &[117, 118, 119],
                    "\"aeroway\":\"holding_position\",\"holding_position:type\":\"intermediate\"",
                ),
            ],
            &[holding(802, "runway"), holding(828, "intermediate")],
        )?;
        let plane = Plane::new(PLANE_ORIGIN);
        let mut junctions: HashSet<i64> = [802, 828].into();
        junctions.extend(hold_lines(&data));
        assert!(junctions.contains(&118) && !junctions.contains(&117));
        let graph = Graph::build(&data, &plane, |_| true, &junctions);
        let runways = [runway(&plane)?];
        let holds = holding_positions(&graph, &data, &runways);
        assert_eq!(holds.len(), 1);
        assert_eq!(graph.nodes[holds[0].node].osm, Some(802));
        assert_eq!(holds[0].runway, "09".parse()?);
        let mut middle: Vec<Option<i64>> = intermediate_holds(&graph, &data)
            .iter()
            .map(|&n| graph.nodes[n].osm)
            .collect();
        middle.sort_unstable();
        assert_eq!(middle, [Some(118), Some(828)]);

        let exits = exits(&graph, &runways);
        let east: Vec<(Option<i64>, f64)> = exits
            .iter()
            .filter(|e| e.runway.to_string() == "09")
            .map(|e| (graph.nodes[e.node].osm, e.angle.round()))
            .collect();
        // Landing eastbound: perpendicular exits at 90°, the north-west one is
        // a 120°+ turn back and not used.
        assert_eq!(east, vec![(Some(2), 90.0), (Some(28), 90.0)]);
        let west: Vec<(Option<i64>, f64)> = exits
            .iter()
            .filter(|e| e.runway.to_string() == "27")
            .map(|e| (graph.nodes[e.node].osm, e.angle.round()))
            .collect();
        assert_eq!(west.first().map(|w| w.0), Some(Some(28)));
        assert!(west.iter().any(|w| w.0 == Some(20) && w.1 < 60.0));
        assert!(
            exits
                .windows(2)
                .all(|w| w[0].runway != w[1].runway || w[0].distance <= w[1].distance)
        );
        Ok(())
    }
}
