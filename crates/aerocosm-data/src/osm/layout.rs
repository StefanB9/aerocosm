//! One airport's OSM data → its ground definition: network, stands, holding
//! positions, exits and aprons, pruned to the part connected to a runway.

use std::collections::{HashSet, VecDeque};

use aerocosm_core::{
    Id,
    ground::{
        AirportGroundDefinition, EdgeDefinition, ExitDefinition, HoldingPositionDefinition,
        StandDefinition,
    },
    units::{Degrees, Meters},
    world::{Airport, World},
};
use anyhow::{Context, Result};

use super::{
    graph::Graph,
    local::{Plane, Xy, inside},
    names::ground_name,
    overpass::{OsmData, OsmWay},
    runways::{LocalRunway, RawExit, RawHolding, exits, holding_positions},
    sizes::StandSizes,
    stands::{Area, RawStand, area_of, attach_stands, wingspan_limits},
};
use crate::geometry::simplify;

/// Apron outlines are simplified to this tolerance, in metres.
const APRON_TOLERANCE: f64 = 2.0;

/// The closed outline of `way`, if it is a ring of known nodes.
fn ring(data: &OsmData, plane: &Plane, way: &OsmWay) -> Option<Vec<Xy>> {
    if way.nodes.len() < 4 || way.nodes.first() != way.nodes.last() {
        return None;
    }
    way.nodes
        .iter()
        .map(|id| data.position(*id).map(|p| plane.project(p)))
        .collect()
}

/// The aerodrome outline: the one tagged with the airport's ICAO code, else
/// the one containing the airport's reference point.
fn aerodrome(data: &OsmData, plane: &Plane, ident: &str) -> Option<Vec<Xy>> {
    let outlines: Vec<(&OsmWay, Vec<Xy>)> = data
        .ways
        .iter()
        .filter(|way| way.aeroway() == Some("aerodrome"))
        .filter_map(|way| ring(data, plane, way).map(|ring| (way, ring)))
        .collect();
    outlines
        .iter()
        .find(|(way, _)| way.tags.get("icao").is_some_and(|icao| icao == ident))
        .or_else(|| outlines.iter().find(|(_, ring)| inside([0.0, 0.0], ring)))
        .map(|(_, ring)| ring.clone())
}

/// Named outlines with the given `aeroway`, e.g. aprons or terminals.
fn areas(data: &OsmData, plane: &Plane, aeroway: &str, prefix: &str) -> Vec<Area> {
    data.ways
        .iter()
        .filter(|way| way.aeroway() == Some(aeroway))
        .filter_map(|way| {
            let label = way.tags.get("ref").or_else(|| way.tags.get("name"))?;
            // Terminal refs are often bare numbers: "1" → "T1".
            let label = if label.chars().all(|c| c.is_ascii_alphanumeric()) && label.len() <= 2 {
                format!("{prefix}{label}")
            } else {
                label.clone()
            };
            Some(Area {
                name: ground_name(&label)?,
                ring: ring(data, plane, way)?,
            })
        })
        .collect()
}

/// Graph nodes connected (either way) to a node on a runway.
fn connected_to_runways(graph: &Graph, runways: &[LocalRunway]) -> Vec<bool> {
    let adjacency = graph.adjacency();
    let mut keep = vec![false; graph.nodes.len()];
    let mut queue: VecDeque<usize> = (0..graph.nodes.len())
        .filter(|&n| runways.iter().any(|r| r.contains(graph.nodes[n].xy)))
        .collect();
    for &n in &queue {
        keep[n] = true;
    }
    while let Some(n) = queue.pop_front() {
        for &edge in &adjacency[n] {
            let next = graph.edges[edge].other(n);
            if !keep[next] {
                keep[next] = true;
                queue.push_back(next);
            }
        }
    }
    keep
}

/// What the import found at one airport, before the quality gate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    pub(crate) definition: AirportGroundDefinition,
    /// Stands left out because they are not connected to a runway.
    pub(crate) dropped_stands: usize,
}

/// Builds `airport`'s ground layout from its OSM download.
pub(crate) fn build_layout(
    data: &OsmData,
    world: &World,
    airport: Id<Airport>,
    sizes: &StandSizes,
) -> Result<Layout> {
    let site = world.airport(airport).context("airport id out of range")?;
    let ident = site.ident();
    let location = site.location();
    let plane = Plane::new([location.latitude().value(), location.longitude().value()]);
    let outline = aerodrome(data, &plane, ident.as_str());
    let keep = |way: &OsmWay| {
        outline.as_ref().is_none_or(|ring| {
            way.nodes
                .iter()
                .filter_map(|id| data.position(*id))
                .any(|p| inside(plane.project(p), ring))
        })
    };
    let junctions: HashSet<i64> = data
        .nodes
        .values()
        .filter(|node| {
            node.tags
                .get("aeroway")
                .is_some_and(|a| a == "holding_position" || a == "parking_position")
        })
        .map(|node| node.id)
        .collect();
    let mut graph = Graph::build(data, &plane, keep, &junctions);
    let stands = attach_stands(&mut graph, data, &plane);
    let runways: Vec<LocalRunway> = world
        .runways_of(airport)
        .iter()
        .map(|runway| LocalRunway::new(runway, &plane))
        .collect();
    let connected = connected_to_runways(&graph, &runways);
    let holdings = holding_positions(&graph, data, &runways);
    let exits = exits(&graph, &runways);

    let aprons = areas(data, &plane, "apron", "");
    let terminals = areas(data, &plane, "terminal", "T");
    let apron_outlines: Vec<Vec<Xy>> = data
        .ways
        .iter()
        .filter(|way| way.aeroway() == Some("apron") && keep(way))
        .filter_map(|way| ring(data, &plane, way))
        .map(|ring| simplify(&ring, APRON_TOLERANCE))
        .collect();

    let total_stands = stands.len();
    let definition = assemble(
        ident,
        &plane,
        &graph,
        &connected,
        &Parts {
            stands: &stands,
            holdings: &holdings,
            exits: &exits,
            aprons: &aprons,
            terminals: &terminals,
            outlines: &apron_outlines,
            sizes,
        },
    )?;
    Ok(Layout {
        dropped_stands: total_stands - definition.stands.len(),
        definition,
    })
}

/// Features found on the graph.
struct Parts<'a> {
    stands: &'a [RawStand],
    holdings: &'a [RawHolding],
    exits: &'a [RawExit],
    aprons: &'a [Area],
    terminals: &'a [Area],
    outlines: &'a [Vec<Xy>],
    sizes: &'a StandSizes,
}

fn degrees(value: f64) -> Result<Degrees> {
    Degrees::new((value * 10.0).round() / 10.0)
}

fn meters(value: f64) -> Result<Meters> {
    Meters::new((value * 10.0).round() / 10.0)
}

/// Renumbers the kept nodes and writes the definition in geographic
/// coordinates.
fn assemble(
    ident: aerocosm_core::world::codes::AirportIdent,
    plane: &Plane,
    graph: &Graph,
    connected: &[bool],
    parts: &Parts<'_>,
) -> Result<AirportGroundDefinition> {
    let mut index = vec![None; graph.nodes.len()];
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for (n, node) in graph.nodes.iter().enumerate() {
        if connected[n] {
            index[n] = Some(u32::try_from(nodes.len()).context("too many ground nodes")?);
            nodes.push(plane.unproject(node.xy));
        }
    }
    let edges = graph
        .edges
        .iter()
        .filter_map(|edge| {
            Some(EdgeDefinition {
                from: index[edge.from]?,
                to: index[edge.to]?,
                kind: edge.kind,
                name: edge.name,
                oneway: edge.oneway,
                max_wingspan: edge.max_wingspan,
                points: edge.points.iter().map(|&p| plane.unproject(p)).collect(),
            })
        })
        .collect();
    let kept: Vec<&RawStand> = parts
        .stands
        .iter()
        .filter(|stand| connected[stand.node])
        .collect();
    let points: Vec<Xy> = kept
        .iter()
        .map(|stand| graph.nodes[stand.node].xy)
        .collect();
    let named: Vec<_> = kept
        .iter()
        .zip(&points)
        .map(|(s, p)| (s.name, *p, s.heading))
        .collect();
    let spans = wingspan_limits(&named);
    let mut stands = Vec::with_capacity(kept.len());
    for ((stand, span), p) in kept.iter().zip(spans).zip(&points) {
        stands.push(StandDefinition {
            name: stand.name,
            node: index[stand.node].context("kept stand without a node")?,
            heading: degrees(stand.heading)?,
            max_wingspan: meters(parts.sizes.limit(ident, stand.name.as_str(), span))?,
            area: area_of(*p, parts.aprons, parts.terminals),
        });
    }
    let holding_positions = parts
        .holdings
        .iter()
        .filter_map(|holding| {
            Some(HoldingPositionDefinition {
                node: index[holding.node]?,
                runway: holding.runway,
            })
        })
        .collect();
    let mut exits = Vec::with_capacity(parts.exits.len());
    for exit in parts.exits {
        if let Some(node) = index[exit.node] {
            exits.push(ExitDefinition {
                runway: exit.runway,
                node,
                distance: meters(exit.distance)?,
                angle: degrees(exit.angle)?,
            });
        }
    }
    let aprons = parts
        .outlines
        .iter()
        .map(|ring| ring.iter().map(|&p| plane.unproject(p)).collect())
        .collect();
    Ok(AirportGroundDefinition {
        airport: ident,
        nodes,
        edges,
        stands,
        holding_positions,
        exits,
        aprons,
    })
}
