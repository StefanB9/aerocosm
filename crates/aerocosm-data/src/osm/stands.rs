//! Parking stands (plan M4 D68): where each `parking_position` ends, which way
//! a parked aircraft faces, how big it may be and which apron it is on.

use std::collections::{HashMap, HashSet};

use aerocosm_core::ground::{CODES, EdgeKind, GroundName, Parked, stand_base};

use super::{
    graph::Graph,
    local::{Plane, Xy, bearing, distance, distance_to_line, inside},
    names::ground_name,
    overpass::{OsmData, Tags},
};

/// Lead-ins and stand nodes farther than this from a taxi lane stay
/// unconnected (and are dropped with their component).
const CONNECT_LIMIT: f64 = 60.0;
/// Stands nearer than this to another are alternative centrelines of one
/// gate: not even the smallest airliners use both at once (code A's 15 m
/// and 3 m, less the mapping tolerance).
const ALTERNATIVE: f64 = 17.0;
/// An aircraft may take the gate on either side of its own with it…
const GATES_TAKEN: usize = 2;
/// …but no more than this many stands in all, however close they lie.
const STANDS_TAKEN: usize = 4;
/// A stand within this distance of a terminal belongs to it.
const TERMINAL_REACH: f64 = 250.0;

/// A stand found in the data, before pruning.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawStand {
    pub(crate) name: GroundName,
    pub(crate) node: usize,
    pub(crate) heading: f64,
}

fn stand_name(tags: &super::overpass::Tags) -> Option<GroundName> {
    tags.get("ref")
        .or_else(|| tags.get("name"))
        .and_then(|text| ground_name(text))
}

/// Whether `edge` is a taxi edge a stand may connect to.
fn taxi(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::Taxiway | EdgeKind::Taxilane)
}

/// Finds the stands of `data` in `graph`, connecting loose lead-ins and stand
/// nodes to the nearest taxi lane. Names are made unique with a suffix.
pub(crate) fn attach_stands(graph: &mut Graph, data: &OsmData, plane: &Plane) -> Vec<RawStand> {
    let mut stands = Vec::new();
    // Named stop nodes that already named a lead-in's stand.
    let mut used: HashSet<i64> = HashSet::new();
    let lead_in_ways: Vec<i64> = data
        .ways
        .iter()
        .filter(|way| way.aeroway() == Some("parking_position"))
        .map(|way| way.id)
        .collect();
    for way_id in lead_in_ways {
        if let Some(stand) = lead_in_stand(graph, data, way_id, &mut used) {
            stands.push(stand);
        }
    }
    let mut free: Vec<_> = data
        .nodes
        .values()
        .filter(|node| {
            node.tags
                .get("aeroway")
                .is_some_and(|a| a == "parking_position")
        })
        .collect();
    free.sort_by_key(|node| node.id);
    for node in free {
        if used.contains(&node.id) {
            continue;
        }
        let Some(name) = stand_name(&node.tags) else {
            continue;
        };
        let xy = plane.project([node.lat, node.lon]);
        if let Some(stand) = node_stand(graph, name, node.id, xy) {
            stands.push(stand);
        }
    }
    unique_names(&mut stands);
    stands
}

/// The name of a lead-in: its own, else that of a named stop node on it
/// (the last such node first), which is then `used`.
fn lead_in_name(
    data: &OsmData,
    way: &super::overpass::OsmWay,
    used: &mut HashSet<i64>,
) -> Option<GroundName> {
    if let Some(name) = stand_name(&way.tags) {
        return Some(name);
    }
    way.nodes.iter().rev().find_map(|id| {
        let node = data.nodes.get(id)?;
        if node
            .tags
            .get("aeroway")
            .is_none_or(|a| a != "parking_position")
        {
            return None;
        }
        let name = stand_name(&node.tags)?;
        used.insert(*id);
        Some(name)
    })
}

/// The stand at the loose end of lead-in way `way_id`.
fn lead_in_stand(
    graph: &mut Graph,
    data: &OsmData,
    way_id: i64,
    used: &mut HashSet<i64>,
) -> Option<RawStand> {
    let way = data.ways.iter().find(|way| way.id == way_id)?;
    let name = lead_in_name(data, way, used)?;
    let first = graph.node_of(*way.nodes.first()?)?;
    let last = graph.node_of(*way.nodes.last()?)?;
    let adjacency = graph.adjacency();
    let joined = |node: usize| {
        adjacency[node]
            .iter()
            .any(|&edge| graph.edges[edge].way != Some(way_id))
    };
    let (open, joined_end) = match (joined(first), joined(last)) {
        (false, true) => (first, last),
        (true, false) => (last, first),
        (true, true) => return drive_through(graph, way_id, name),
        (false, false) => {
            // Connect whichever end is nearer a taxi lane.
            let near = |node: usize| {
                graph
                    .nearest_on_edges(graph.nodes[node].xy, CONNECT_LIMIT, |_, edge| {
                        taxi(edge.kind)
                    })
                    .map(|found| found.3)
            };
            let (open, connect) = match (near(first), near(last)) {
                (Some(a), Some(b)) if b < a => (first, last),
                (Some(_), _) => (last, first),
                (None, Some(_)) => (first, last),
                (None, None) => return None,
            };
            connect_node(graph, connect)?;
            (open, connect)
        }
    };
    // A stop node of the same name at the stand end is this stand again.
    if let Some(id) = graph.nodes[open].osm
        && data
            .nodes
            .get(&id)
            .and_then(|node| stand_name(&node.tags))
            .is_some_and(|node_name| node_name == name)
    {
        used.insert(id);
    }
    // Facing the stand end along the lead-in's last stretch.
    let edge = adjacency[open].first().copied()?;
    let before = graph.edges[edge].next_after(open, &graph.nodes);
    let heading = if distance(before, graph.nodes[open].xy) > 0.01 {
        bearing(before, graph.nodes[open].xy)
    } else {
        bearing(graph.nodes[joined_end].xy, graph.nodes[open].xy)
    };
    Some(RawStand {
        name,
        node: open,
        heading,
    })
}

/// A stand line joined at both ends: aircraft taxi through and park at its
/// middle, facing along the way.
fn drive_through(graph: &mut Graph, way_id: i64, name: GroundName) -> Option<RawStand> {
    let edge = (0..graph.edges.len())
        .filter(|&e| graph.edges[e].way == Some(way_id))
        .max_by(|&a, &b| {
            line_length(&graph.edges[a].polyline(&graph.nodes))
                .total_cmp(&line_length(&graph.edges[b].polyline(&graph.nodes)))
        })?;
    let line = graph.edges[edge].polyline(&graph.nodes);
    let half = line_length(&line) / 2.0;
    let mut walked = 0.0;
    for (segment, pair) in line.windows(2).enumerate() {
        let step = distance(pair[0], pair[1]);
        if walked + step >= half && step > 0.0 {
            let t = (half - walked) / step;
            let point = [
                pair[0][0] + t * (pair[1][0] - pair[0][0]),
                pair[0][1] + t * (pair[1][1] - pair[0][1]),
            ];
            let heading = bearing(pair[0], pair[1]);
            let node = graph.split_edge(edge, segment, point);
            return Some(RawStand {
                name,
                node,
                heading,
            });
        }
        walked += step;
    }
    None
}

fn line_length(line: &[Xy]) -> f64 {
    line.windows(2).map(|pair| distance(pair[0], pair[1])).sum()
}

/// Connects `node` to the nearest point of a taxi lane with a lead-in.
fn connect_node(graph: &mut Graph, node: usize) -> Option<usize> {
    let xy = graph.nodes[node].xy;
    let (edge, segment, point, _) =
        graph.nearest_on_edges(xy, CONNECT_LIMIT, |_, edge| taxi(edge.kind))?;
    let joint = graph.split_edge(edge, segment, point);
    if joint != node {
        graph.connect(joint, node, EdgeKind::LeadIn);
    }
    Some(joint)
}

/// A stand mapped as a single node: on the network, facing along it, or
/// beside it, connected by a lead-in.
fn node_stand(graph: &mut Graph, name: GroundName, id: i64, xy: Xy) -> Option<RawStand> {
    if let Some(node) = graph.node_of(id) {
        let edge = graph.adjacency()[node].first().copied()?;
        let ahead = graph.edges[edge].next_after(node, &graph.nodes);
        return Some(RawStand {
            name,
            node,
            heading: bearing(xy, ahead),
        });
    }
    let node = graph.nodes.len();
    graph
        .nodes
        .push(super::graph::GraphNode { xy, osm: Some(id) });
    let Some(joint) = connect_node(graph, node) else {
        graph.nodes.pop();
        return None;
    };
    Some(RawStand {
        name,
        node,
        heading: bearing(graph.nodes[joint].xy, xy),
    })
}

/// Appends `-2`, `-3`, … to repeated names (by order of appearance).
fn unique_names(stands: &mut [RawStand]) {
    let mut seen: HashMap<GroundName, u32> = HashMap::new();
    let taken: HashSet<GroundName> = stands.iter().map(|stand| stand.name).collect();
    for stand in stands.iter_mut() {
        let count = seen.entry(stand.name).or_default();
        *count += 1;
        if *count == 1 {
            continue;
        }
        let base = stand.name.as_str().to_owned();
        let mut suffix = *count;
        loop {
            let tail = format!("-{suffix}");
            let room = 16_usize.saturating_sub(tail.len());
            let candidate: String = base.chars().take(room).collect::<String>() + &tail;
            if let Some(name) = ground_name(&candidate)
                && !taken.contains(&name)
                && !seen.contains_key(&name)
            {
                seen.insert(name, 1);
                stand.name = name;
                break;
            }
            suffix += 1;
        }
    }
}

/// Wingspan limit of each stand (name, position, heading), m: the largest
/// ICAO aerodrome code (A–F) that fits it with the stands beside it empty
/// (plan M6 D148). An aircraft that size may reach over the gate on either
/// side, the alternative centrelines of a gate counting once and those of
/// its own gate not at all, but over no more than a few stands. Which
/// stands are free beside a parked aircraft is decided when one arrives,
/// from the aircraft's actual sizes.
pub(crate) fn wingspan_limits(stands: &[(GroundName, Xy, f64)]) -> Vec<f64> {
    stands
        .iter()
        .map(|(name, p, heading)| {
            let base = stand_base(name.as_str());
            let fits = |span: f64| {
                let me = Parked {
                    at: *p,
                    heading: *heading,
                    wingspan: span,
                };
                let taken: Vec<Xy> = stands
                    .iter()
                    .filter(|(other, q, _)| {
                        stand_base(other.as_str()) != base && distance(*p, *q) >= ALTERNATIVE
                    })
                    .filter(|(_, q, other_heading)| {
                        !me.keeps_clear_of(&Parked {
                            at: *q,
                            heading: *other_heading,
                            wingspan: span,
                        })
                    })
                    .map(|(_, q, _)| *q)
                    .collect();
                taken.len() <= STANDS_TAKEN && gates(&taken) <= GATES_TAKEN
            };
            CODES
                .iter()
                .rev()
                .find(|(span, _)| fits(*span))
                .map_or(CODES[0].0, |(span, _)| *span)
        })
        .collect()
}

/// How many gates the stand points `stands` belong to: points nearer than
/// `ALTERNATIVE` to each other are centrelines of one gate.
fn gates(stands: &[Xy]) -> usize {
    let mut gate: Vec<usize> = (0..stands.len()).collect();
    for i in 0..stands.len() {
        for j in 0..i {
            if distance(stands[i], stands[j]) < ALTERNATIVE {
                let (from, to) = (gate[i], gate[j]);
                for g in &mut gate {
                    if *g == from {
                        *g = to;
                    }
                }
            }
        }
    }
    gate.sort_unstable();
    gate.dedup();
    gate.len()
}

/// A named apron or terminal outline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Area {
    pub(crate) name: GroundName,
    pub(crate) ring: Vec<Xy>,
}

/// Whether an apron's OSM tags mark it for cargo (plan M5 D120): `apron`
/// or `usage` set to cargo, or the word in its name, ref or description.
pub(crate) fn marks_cargo(tags: &Tags) -> bool {
    let set = |key: &str| {
        tags.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("cargo"))
    };
    set("apron")
        || set("usage")
        || ["name", "ref", "description"].iter().any(|key| {
            tags.get(*key)
                .is_some_and(|v| v.to_ascii_lowercase().contains("cargo"))
        })
}

/// The apron containing `p`, else the nearest terminal within reach.
pub(crate) fn area_of(p: Xy, aprons: &[Area], terminals: &[Area]) -> Option<GroundName> {
    if let Some(apron) = aprons.iter().find(|apron| inside(p, &apron.ring)) {
        return Some(apron.name);
    }
    terminals
        .iter()
        .map(|terminal| (distance_to_line(p, &terminal.ring), terminal.name))
        .filter(|(d, _)| *d <= TERMINAL_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use anyhow::{Context, Result};

    use super::*;
    use crate::osm::graph::tests::{PLANE_ORIGIN, data, grid_node, way};

    fn plane() -> Plane {
        Plane::new(PLANE_ORIGIN)
    }

    #[test]
    fn test_lead_ins_end_in_stands_facing_inwards() -> Result<()> {
        // A lane along row 0; one lead-in joined to it at node 2 going north,
        // one loose lead-in north of node 5 (20 m away), one far away.
        let data = data(
            &[
                way(1, &[0, 1, 2, 3, 4, 5, 6], "\"aeroway\":\"taxilane\""),
                way(
                    2,
                    &[2, 102, 202],
                    "\"aeroway\":\"parking_position\",\"ref\":\"A1\"",
                ),
                way(
                    3,
                    &[205, 305],
                    "\"aeroway\":\"parking_position\",\"ref\":\"A2\"",
                ),
                way(
                    4,
                    &[2090, 2190],
                    "\"aeroway\":\"parking_position\",\"ref\":\"A3\"",
                ),
            ],
            &[],
        )?;
        let plane = plane();
        let mut graph = Graph::build(&data, &plane, |_| true, &HashSet::new());
        let stands = attach_stands(&mut graph, &data, &plane);
        assert_eq!(stands.len(), 2);
        let a1 = &stands[0];
        assert_eq!(a1.name.as_str(), "A1");
        assert_eq!(graph.nodes[a1.node].osm, Some(202));
        assert!(a1.heading < 1.0 || a1.heading > 359.0, "{}", a1.heading);
        let a2 = &stands[1];
        assert_eq!(graph.nodes[a2.node].osm, Some(305));
        // The loose lead-in got a connector to the lane.
        let connectors = graph.edges.iter().filter(|e| e.way.is_none()).count();
        assert_eq!(connectors, 1);
        Ok(())
    }

    #[test]
    fn test_unnamed_lead_ins_take_the_name_of_their_stand_node() -> Result<()> {
        let tagged = |id: i64, name: &str| {
            let node = grid_node(id);
            format!(
                "{},\"tags\":{{\"aeroway\":\"parking_position\",\"ref\":\"{name}\"}}}}",
                &node[..node.len() - 1]
            )
        };
        // An unnamed lead-in north from node 2 with its stop node named V01;
        // a named stop node on the lane itself at node 5.
        let data = data(
            &[
                way(1, &[0, 1, 2, 3, 4, 5, 6], "\"aeroway\":\"taxilane\""),
                way(2, &[2, 102, 202], "\"aeroway\":\"parking_position\""),
            ],
            &[tagged(202, "V01"), tagged(5, "V02")],
        )?;
        let plane = plane();
        let junctions: HashSet<i64> = [202, 5].into();
        let mut graph = Graph::build(&data, &plane, |_| true, &junctions);
        let stands = attach_stands(&mut graph, &data, &plane);
        let names: Vec<&str> = stands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["V01", "V02"]);
        assert_eq!(graph.nodes[stands[0].node].osm, Some(202));
        assert_eq!(graph.nodes[stands[1].node].osm, Some(5));
        Ok(())
    }

    #[test]
    fn test_stand_nodes_ending_a_named_lead_in_are_one_stand() -> Result<()> {
        let tagged = |id: i64, name: &str| {
            let node = grid_node(id);
            format!(
                "{},\"tags\":{{\"aeroway\":\"parking_position\",\"ref\":\"{name}\"}}}}",
                &node[..node.len() - 1]
            )
        };
        // A lead-in named 4 north from node 2, ending at a stop node also
        // named 4 (as at Weeze).
        let data = data(
            &[
                way(1, &[0, 1, 2, 3, 4], "\"aeroway\":\"taxilane\""),
                way(
                    2,
                    &[2, 102, 202],
                    "\"aeroway\":\"parking_position\",\"ref\":\"4\"",
                ),
            ],
            &[tagged(202, "4")],
        )?;
        let plane = plane();
        let junctions: HashSet<i64> = [202].into();
        let mut graph = Graph::build(&data, &plane, |_| true, &junctions);
        let stands = attach_stands(&mut graph, &data, &plane);
        let names: Vec<&str> = stands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["4"]);
        assert!(stands[0].heading < 1.0 || stands[0].heading > 359.0);
        Ok(())
    }

    #[test]
    fn test_drive_through_stands_park_mid_line() -> Result<()> {
        // Lanes along rows 0 and 10; a stand line from one to the other.
        let data = data(
            &[
                way(1, &[0, 3, 6], "\"aeroway\":\"taxilane\""),
                way(2, &[1000, 1003, 1006], "\"aeroway\":\"taxilane\""),
                way(
                    3,
                    &[3, 1003],
                    "\"aeroway\":\"parking_position\",\"ref\":\"71\"",
                ),
            ],
            &[],
        )?;
        let plane = plane();
        let mut graph = Graph::build(&data, &plane, |_| true, &HashSet::new());
        let stands = attach_stands(&mut graph, &data, &plane);
        let stand = stands.first().context("a drive-through stand")?;
        assert_eq!(stand.name.as_str(), "71");
        let at = graph.nodes[stand.node].xy;
        let expected = plane.project([50.0005, 8.5 + 3.0 * 0.00014]);
        assert!(distance(at, expected) < 0.5, "{at:?} vs {expected:?}");
        assert!(
            stand.heading < 1.0 || stand.heading > 359.0,
            "{}",
            stand.heading
        );
        // Still a line through: two lead-in halves meet at the stand.
        assert_eq!(graph.adjacency()[stand.node].len(), 2);
        Ok(())
    }

    #[test]
    fn test_stand_nodes_connect_and_names_stay_unique() -> Result<()> {
        let stand = |id: i64, name: &str| {
            let node = grid_node(id);
            format!(
                "{},\"tags\":{{\"aeroway\":\"parking_position\",\"ref\":\"{name}\"}}}}",
                &node[..node.len() - 1]
            )
        };
        let data = data(
            &[way(1, &[0, 1, 2, 3], "\"aeroway\":\"taxiway\"")],
            &[stand(201, "B1"), stand(203, "B1"), stand(5000, "B9")],
        )?;
        let plane = plane();
        let mut graph = Graph::build(&data, &plane, |_| true, &HashSet::new());
        let stands = attach_stands(&mut graph, &data, &plane);
        let names: Vec<&str> = stands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["B1", "B1-2"]);
        let first = stands.first().context("a stand")?;
        // South of the stand is the taxiway: the aircraft faces north.
        assert!(first.heading < 1.0 || first.heading > 359.0);
        Ok(())
    }

    /// Stands facing north.
    fn named(points: &[(&str, Xy)]) -> Result<Vec<(GroundName, Xy, f64)>> {
        points
            .iter()
            .map(|(name, xy)| Ok((name.parse()?, *xy, 0.0)))
            .collect()
    }

    /// A row of `count` stands `spacing` metres apart, all heading north.
    fn row(count: u32, spacing: f64) -> Result<Vec<(GroundName, Xy, f64)>> {
        (0..count)
            .map(|i| {
                Ok((
                    format!("{}", i + 1).parse()?,
                    [f64::from(i) * spacing, 0.0],
                    0.0,
                ))
            })
            .collect()
    }

    #[test]
    fn test_wingspan_limits_let_a_stand_take_the_gates_beside_it() -> Result<()> {
        // Gates 40 m apart, built for the 737: with the gate on either side
        // empty a code E wide-body fits, a code F would reach the next two.
        // At the row's ends there is room on one side.
        assert_eq!(
            wingspan_limits(&row(5, 40.0)?),
            vec![80.0, 65.0, 65.0, 65.0, 80.0]
        );
        // 34 m apart (Ankara): every other stand takes an A320, code D at
        // the most.
        assert_eq!(wingspan_limits(&row(5, 34.0)?)[2], 52.0);
        // Alone, or with one neighbour: nothing limits it.
        assert_eq!(wingspan_limits(&row(2, 45.0)?), vec![80.0, 80.0]);
        Ok(())
    }

    #[test]
    fn test_wingspan_limits_count_the_alternatives_of_a_gate_once() -> Result<()> {
        // Gates 60 m apart, each with a centreline for wide-bodies and one
        // 12 m to either side for narrow-bodies, under names of their own.
        let mut stands = Vec::new();
        for gate in 0..5_u32 {
            for (suffix, offset) in [("L", -12.0), ("C", 0.0), ("R", 12.0)] {
                let x = f64::from(gate) * 60.0 + offset;
                stands.push((format!("G{gate}{suffix}").parse()?, [x, 0.0], 0.0));
            }
        }
        let limits = wingspan_limits(&stands);
        // The middle gate's centreline: code E, the gate on either side
        // still usable on its far line.
        assert!((limits[7] - 65.0).abs() < 1e-9, "{limits:?}");
        Ok(())
    }

    #[test]
    fn test_wingspan_limits_keep_large_aircraft_off_crowded_aprons() -> Result<()> {
        // Tie-downs 12 m apart: an airliner would take six of them.
        let limits = wingspan_limits(&row(12, 12.0)?);
        assert!(limits[6] <= 36.0, "{limits:?}");
        Ok(())
    }

    #[test]
    fn test_wingspan_limits_ignore_variants_of_the_same_stand() -> Result<()> {
        // 05 with its narrow variants 05A and 05B, then 06 80 m away.
        let limits = wingspan_limits(&named(&[
            ("05", [0.0, 0.0]),
            ("05A", [-20.0, 0.0]),
            ("05B", [20.0, 0.0]),
            ("06", [100.0, 0.0]),
            ("N4", [500.0, 0.0]),
            ("N4-2", [505.0, 0.0]),
        ])?);
        assert_eq!(limits, vec![80.0; 6]);
        Ok(())
    }

    #[test]
    fn test_area_prefers_the_apron_then_the_nearest_terminal() -> Result<()> {
        let square = |x: f64, name: &str| -> Result<Area> {
            Ok(Area {
                name: name.parse()?,
                ring: vec![
                    [x, 0.0],
                    [x + 100.0, 0.0],
                    [x + 100.0, 100.0],
                    [x, 100.0],
                    [x, 0.0],
                ],
            })
        };
        let aprons = [square(0.0, "Apron V")?];
        let terminals = [square(300.0, "T1")?, square(-500.0, "T2")?];
        assert_eq!(
            area_of([50.0, 50.0], &aprons, &terminals).map(|n| n.to_string()),
            Some("Apron V".to_owned())
        );
        assert_eq!(
            area_of([200.0, 50.0], &aprons, &terminals).map(|n| n.to_string()),
            Some("T1".to_owned())
        );
        assert_eq!(area_of([200.0, 900.0], &aprons, &terminals), None);
        Ok(())
    }

    #[test]
    fn test_marks_cargo_by_tag_or_name() {
        let tags = |pairs: &[(&str, &str)]| -> Tags {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        assert!(marks_cargo(&tags(&[("apron", "cargo")])));
        assert!(marks_cargo(&tags(&[("usage", "Cargo")])));
        assert!(marks_cargo(&tags(&[("name", "CargoCity Süd")])));
        assert!(marks_cargo(&tags(&[("description", "DHL cargo apron")])));
        assert!(!marks_cargo(&tags(&[("name", "Vorfeld V")])));
        assert!(!marks_cargo(&tags(&[])));
    }
}
