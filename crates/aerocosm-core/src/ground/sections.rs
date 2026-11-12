//! Intersections, runways and the stretches between them (plan M4 D89,
//! D93): the parts of an airport's network that ground control works with.
//!
//! A ground controller sequences *areas*: taxiway intersections and
//! runways, one aircraft at a time, entered from a holding position. On the
//! *stretches* between areas pilots keep apart by following each other.
//!
//! An area reaches from its junction out to the holding position on each
//! approach: the real one where the layout maps it (an intermediate hold
//! before an intersection, a runway holding position before a runway), else
//! one synthesised at a fixed distance. Junctions too close together for an
//! aircraft to stop between them form one area. Stand lead-ins belong to no
//! section: an aircraft on one is at its stand.

use super::{AirportGround, Edge, EdgeKind, Node};
use crate::Id;

/// A synthesised hold this far from its intersection's junction, m: a
/// waiting aircraft's nose keeps its wings clear of the crossing taxiway.
const INTERSECTION_HOLD: f64 = 45.0;
/// A synthesised hold this far from a runway node, m: clear of the runway
/// strip.
const RUNWAY_HOLD: f64 = 90.0;
/// A stretch is at least this long, m: one aircraft fits on it.
const MIN_STRETCH: f64 = 50.0;

/// What a piece of the network belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Section {
    /// An intersection or runway area, by index.
    Area(u32),
    /// A stretch between areas, by index.
    Stretch(u32),
}

/// A piece of an edge, from `from` to `to` metres along it from its `from`
/// node, in one section; on a stretch also where the piece starts in the
/// stretch's own coordinate and whether it runs the stretch's way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Piece {
    pub(crate) from: f64,
    pub(crate) to: f64,
    pub(crate) section: Section,
    pub(crate) at: f64,
    pub(crate) forward: bool,
}

/// One area.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Area {
    /// Whether it is (or holds) a runway.
    pub(crate) runway: bool,
    /// Its junction or runway nodes.
    pub(crate) nodes: Vec<Id<Node>>,
}

/// An airport's areas and stretches.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sections {
    /// Per edge, its pieces in order from its `from` node; empty for a
    /// lead-in.
    pieces: Vec<Vec<Piece>>,
    areas: Vec<Area>,
    /// Per stretch, its length, m.
    stretches: Vec<f64>,
}

/// One section along a route: entered `from` and left `to` metres along
/// the aircraft's path; on a stretch, where it enters in the stretch's
/// coordinate and whether it runs the stretch's way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Leg {
    pub(crate) section: Section,
    pub(crate) from: f64,
    pub(crate) to: f64,
    pub(crate) at: f64,
    pub(crate) forward: bool,
}

/// A chain of edges between junctions, runway nodes or dead ends, through
/// nodes where nothing branches.
struct Chain {
    edges: Vec<(Id<Edge>, bool)>,
    length: f64,
    start: Option<Id<Node>>,
    end: Option<Id<Node>>,
    /// Holds along it: distance from its start.
    holds: Vec<f64>,
}

/// Union-find over node indices.
struct Merge(Vec<usize>);

impl Merge {
    fn root(&mut self, node: usize) -> usize {
        let mut current = node;
        while self.0[current] != current {
            self.0[current] = self.0[self.0[current]];
            current = self.0[current];
        }
        current
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

impl Sections {
    /// The sections of `layout`.
    pub(crate) fn new(layout: &AirportGround) -> Self {
        let count = layout.nodes().len();
        let kind = |e: Id<Edge>| layout.edge(e).map(Edge::kind);
        let taxiways = |n: usize| {
            Id::<Node>::from_usize(n).map_or(0, |id| {
                layout
                    .edges_at(id)
                    .iter()
                    .filter(|&&e| matches!(kind(e), Some(EdgeKind::Taxiway | EdgeKind::Taxilane)))
                    .count()
            })
        };
        let on_runway: Vec<bool> = (0..count)
            .map(|n| {
                Id::<Node>::from_usize(n).is_ok_and(|id| {
                    layout
                        .edges_at(id)
                        .iter()
                        .any(|&e| kind(e) == Some(EdgeKind::Runway))
                })
            })
            .collect();
        let special: Vec<bool> = (0..count)
            .map(|n| on_runway[n] || taxiways(n) >= 3)
            .collect();
        let mut hold = vec![false; count];
        for &node in layout.intermediate_holds() {
            hold[node.as_usize()] = true;
        }
        for position in layout.holding_positions() {
            hold[position.node().as_usize()] = true;
        }
        let mut merge = Merge((0..count).collect());
        // A runway's nodes are one area.
        for edge in layout.edges() {
            if edge.kind() == EdgeKind::Runway {
                merge.join(edge.from().as_usize(), edge.to().as_usize());
            }
        }
        let chains = chains(layout, &special, &hold);
        // Where a chain ends at an area, how far out its hold is; `None`
        // where the chain has no room for a stretch.
        let mut bounds = Vec::with_capacity(chains.len());
        for chain in &chains {
            let reach = |end: Option<Id<Node>>, near: &dyn Fn(f64) -> f64| -> f64 {
                let Some(end) = end else {
                    return 0.0;
                };
                // A real hold faces the end it is nearer to.
                let real = chain
                    .holds
                    .iter()
                    .map(|&d| near(d))
                    .filter(|&d| d <= chain.length / 2.0)
                    .fold(f64::INFINITY, f64::min);
                if real.is_finite() {
                    real
                } else if on_runway[end.as_usize()] {
                    RUNWAY_HOLD
                } else {
                    INTERSECTION_HOLD
                }
            };
            let start = reach(chain.start, &|d| d);
            let end = reach(chain.end, &|d| chain.length - d);
            if start + end + MIN_STRETCH <= chain.length {
                bounds.push(Some((start, end)));
            } else {
                if let (Some(a), Some(b)) = (chain.start, chain.end) {
                    merge.join(a.as_usize(), b.as_usize());
                }
                bounds.push(None);
            }
        }
        // Areas, numbered by their lowest node.
        let mut area_of = vec![None; count];
        let mut areas: Vec<Area> = Vec::new();
        for node in 0..count {
            if !special[node] {
                continue;
            }
            let root = merge.root(node);
            let index = match area_of[root] {
                Some(index) => index,
                None => {
                    let index = u32::try_from(areas.len()).unwrap_or(u32::MAX);
                    areas.push(Area {
                        runway: false,
                        nodes: Vec::new(),
                    });
                    area_of[root] = Some(index);
                    index
                }
            };
            area_of[node] = Some(index);
            if let (Some(area), Ok(id)) = (areas.get_mut(index as usize), Id::from_usize(node)) {
                area.runway |= on_runway[node];
                area.nodes.push(id);
            }
        }
        let area = |node: Option<Id<Node>>| node.and_then(|n| area_of[n.as_usize()]);
        let mut pieces = vec![Vec::new(); layout.edges().len()];
        for edge in layout.edges().iter().enumerate() {
            if edge.1.kind() == EdgeKind::Runway {
                let index = area(Some(edge.1.from()));
                if let Some(index) = index {
                    pieces[edge.0].push(Piece {
                        from: 0.0,
                        to: edge.1.length().value(),
                        section: Section::Area(index),
                        at: 0.0,
                        forward: true,
                    });
                }
            }
        }
        let mut stretches = Vec::new();
        for (chain, bound) in chains.iter().zip(&bounds) {
            // The chain's parts, in chain distance: start area, stretch, end
            // area.
            let mut parts: Vec<(f64, f64, Section)> = Vec::with_capacity(3);
            match bound {
                Some((start, end)) => {
                    if let Some(index) = area(chain.start).filter(|_| *start > 0.0) {
                        parts.push((0.0, *start, Section::Area(index)));
                    }
                    let index = u32::try_from(stretches.len()).unwrap_or(u32::MAX);
                    stretches.push(chain.length - start - end);
                    parts.push((*start, chain.length - end, Section::Stretch(index)));
                    if let Some(index) = area(chain.end).filter(|_| *end > 0.0) {
                        parts.push((chain.length - end, chain.length, Section::Area(index)));
                    }
                }
                None => {
                    if let Some(index) = area(chain.start).or_else(|| area(chain.end)) {
                        parts.push((0.0, chain.length, Section::Area(index)));
                    }
                }
            }
            let stretch_start = bound.map_or(0.0, |(start, _)| start);
            let mut walked = 0.0;
            for &(id, forward) in &chain.edges {
                let Some(edge) = layout.edge(id) else {
                    continue;
                };
                let length = edge.length().value();
                for &(a, b, section) in &parts {
                    let (a, b) = (a.max(walked), b.min(walked + length));
                    if b <= a {
                        continue;
                    }
                    // In edge distance from its `from` node.
                    let (from, to) = if forward {
                        (a - walked, b - walked)
                    } else {
                        (walked + length - b, walked + length - a)
                    };
                    let at = if forward { a } else { b } - stretch_start;
                    pieces[id.as_usize()].push(Piece {
                        from,
                        to,
                        section,
                        at,
                        forward,
                    });
                }
                walked += length;
            }
        }
        for list in &mut pieces {
            list.sort_by(|a, b| a.from.total_cmp(&b.from));
        }
        Self {
            pieces,
            areas,
            stretches,
        }
    }

    /// The areas.
    pub(crate) fn areas(&self) -> &[Area] {
        &self.areas
    }

    /// Each stretch's length, m.
    pub(crate) fn stretches(&self) -> &[f64] {
        &self.stretches
    }

    /// The sections a route along `edges` passes, from `offset` metres
    /// along the aircraft's path; consecutive pieces of one section are one
    /// leg.
    pub(crate) fn legs(
        &self,
        layout: &AirportGround,
        edges: &[(Id<Edge>, bool)],
        offset: f64,
    ) -> Vec<Leg> {
        let mut legs: Vec<Leg> = Vec::with_capacity(edges.len());
        let mut along = offset;
        for &(id, forward) in edges {
            let Some(edge) = layout.edge(id) else {
                continue;
            };
            let length = edge.length().value();
            let pieces = self
                .pieces
                .get(id.as_usize())
                .map_or(&[][..], Vec::as_slice);
            let count = pieces.len();
            for k in 0..count {
                let piece = &pieces[if forward { k } else { count - 1 - k }];
                let (from, to) = if forward {
                    (along + piece.from, along + piece.to)
                } else {
                    (along + length - piece.to, along + length - piece.from)
                };
                let way = piece.forward == forward;
                // Where the aircraft enters the piece, in stretch distance.
                let at = if way {
                    piece.at
                } else {
                    piece.at + (piece.to - piece.from)
                };
                match legs.last_mut() {
                    Some(last)
                        if last.section == piece.section && (last.to - from).abs() < 1e-6 =>
                    {
                        last.to = to;
                    }
                    _ => legs.push(Leg {
                        section: piece.section,
                        from,
                        to,
                        at,
                        forward: way,
                    }),
                }
            }
            along += length;
        }
        legs
    }
}

/// The chains of `layout`'s taxiways and taxilanes between `special` nodes
/// (or dead ends), each walked once, with the `hold` nodes along them.
fn chains(layout: &AirportGround, special: &[bool], hold: &[bool]) -> Vec<Chain> {
    let usable = |e: Id<Edge>| {
        layout
            .edge(e)
            .is_some_and(|e| matches!(e.kind(), EdgeKind::Taxiway | EdgeKind::Taxilane))
    };
    let degree = |n: Id<Node>| layout.edges_at(n).iter().filter(|&&e| usable(e)).count();
    let mut seen = vec![false; layout.edges().len()];
    let mut chains = Vec::new();
    // Start from special nodes and dead ends first, then any left (loops).
    let mut starts: Vec<Id<Node>> = (0..layout.nodes().len())
        .filter_map(|n| Id::from_usize(n).ok())
        .filter(|&n| special[n.as_usize()] || degree(n) == 1)
        .collect();
    starts.extend((0..layout.nodes().len()).filter_map(|n| Id::from_usize(n).ok()));
    for start in starts {
        for &first in layout.edges_at(start) {
            if seen[first.as_usize()] || !usable(first) {
                continue;
            }
            let mut chain = Chain {
                edges: Vec::new(),
                length: 0.0,
                start: special[start.as_usize()].then_some(start),
                end: None,
                holds: Vec::new(),
            };
            let (mut node, mut edge) = (start, first);
            loop {
                seen[edge.as_usize()] = true;
                let Some(e) = layout.edge(edge) else {
                    break;
                };
                let forward = e.from() == node;
                node = if forward { e.to() } else { e.from() };
                chain.edges.push((edge, forward));
                chain.length += e.length().value();
                if special[node.as_usize()] {
                    chain.end = Some(node);
                    break;
                }
                if hold[node.as_usize()] {
                    chain.holds.push(chain.length);
                }
                let next = layout
                    .edges_at(node)
                    .iter()
                    .copied()
                    .find(|&e| usable(e) && !seen[e.as_usize()]);
                match next {
                    Some(next) if degree(node) == 2 => edge = next,
                    _ => break,
                }
            }
            chains.push(chain);
        }
    }
    chains
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        ground::{
            AirportGroundDefinition, EdgeDefinition, GroundNetwork, HoldingPositionDefinition,
            definition::Point,
        },
        test_data::data,
    };

    /// A layout near Frankfurt from `nodes` (metres east, north of a corner)
    /// and `edges` (from, to, kind), with intermediate holds `holds` and a
    /// runway hold for 25C at `runway_holds`.
    fn layout(
        nodes: &[(f64, f64)],
        edges: &[(u32, u32, EdgeKind)],
        holds: &[u32],
        runway_holds: &[u32],
    ) -> Result<AirportGround> {
        let points: Vec<Point> = nodes
            .iter()
            .map(|&(east, north)| [50.03 + north / 111_195.0, 8.56 + east / 71_470.0])
            .collect();
        AirportGround::new(
            AirportGroundDefinition {
                airport: "EDDF".parse()?,
                nodes: points,
                edges: edges
                    .iter()
                    .map(|&(from, to, kind)| EdgeDefinition {
                        from,
                        to,
                        kind,
                        name: None,
                        oneway: false,
                        max_wingspan: None,
                        points: Vec::new(),
                    })
                    .collect(),
                stands: Vec::new(),
                holding_positions: runway_holds
                    .iter()
                    .map(|&node| {
                        Ok(HoldingPositionDefinition {
                            node,
                            runway: "25C".parse()?,
                        })
                    })
                    .collect::<Result<_>>()?,
                exits: Vec::new(),
                intermediate_holds: holds.to_vec(),
                aprons: Vec::new(),
            },
            &data()?.world,
        )
    }

    const TAXIWAY: EdgeKind = EdgeKind::Taxiway;

    #[test]
    fn test_a_crossing_is_an_area_with_stretches_out_to_its_holds() -> Result<()> {
        // A cross: junction 0 with four arms of 300 m.
        let ground = layout(
            &[
                (0.0, 0.0),
                (300.0, 0.0),
                (-300.0, 0.0),
                (0.0, 300.0),
                (0.0, -300.0),
            ],
            &[
                (0, 1, TAXIWAY),
                (0, 2, TAXIWAY),
                (0, 3, TAXIWAY),
                (0, 4, TAXIWAY),
            ],
            &[],
            &[],
        )?;
        let sections = Sections::new(&ground);
        assert_eq!(sections.areas().len(), 1);
        assert_eq!(sections.areas()[0].nodes, [Id::new(0)]);
        assert_eq!(sections.stretches().len(), 4);
        // Synthesised holds 45 m out; the arms are dead ends.
        assert!(
            sections
                .stretches()
                .iter()
                .all(|&l| (l - 255.0).abs() < 1.0)
        );
        // From arm 1 across to arm 2: stretch, area, stretch.
        let route = [(Id::new(0), false), (Id::new(1), true)];
        let legs = sections.legs(&ground, &route, 10.0);
        let kinds: Vec<Section> = legs.iter().map(|l| l.section).collect();
        assert_eq!(kinds.len(), 3);
        assert_eq!(kinds[1], Section::Area(0));
        assert!((legs[0].from - 10.0).abs() < 1e-6);
        assert!((legs[1].from - 265.0).abs() < 1.0 && (legs[1].to - 355.0).abs() < 1.0);
        assert!((legs[2].to - 610.0).abs() < 1.0);
        // Arm 1 is drawn from the junction out, so it is taxied against
        // its way, entering at its far end.
        assert!(!legs[0].forward && (legs[0].at - 255.0).abs() < 1.0);
        assert!(legs[2].forward && legs[2].at.abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn test_a_real_hold_bounds_its_intersection() -> Result<()> {
        // Junction 0, an arm through node 1 (a mapped hold 80 m out) to 2.
        let ground = layout(
            &[
                (0.0, 0.0),
                (80.0, 0.0),
                (300.0, 0.0),
                (0.0, 300.0),
                (0.0, -300.0),
            ],
            &[
                (0, 1, TAXIWAY),
                (1, 2, TAXIWAY),
                (0, 3, TAXIWAY),
                (0, 4, TAXIWAY),
            ],
            &[1],
            &[],
        )?;
        let sections = Sections::new(&ground);
        let legs = sections.legs(&ground, &[(Id::new(1), false), (Id::new(0), false)], 0.0);
        // From 2 towards the junction: the stretch ends at the hold.
        assert!(matches!(legs[0].section, Section::Stretch(_)));
        assert!((legs[0].to - 220.0).abs() < 1.0, "{legs:?}");
        assert_eq!(legs[1].section, Section::Area(0));
        Ok(())
    }

    #[test]
    fn test_junctions_too_close_for_a_stretch_form_one_area() -> Result<()> {
        // Two junctions 100 m apart: 45 + 45 + 50 does not fit.
        let ground = layout(
            &[
                (0.0, 0.0),
                (100.0, 0.0),
                (0.0, 300.0),
                (0.0, -300.0),
                (100.0, 300.0),
                (100.0, -300.0),
            ],
            &[
                (0, 1, TAXIWAY),
                (0, 2, TAXIWAY),
                (0, 3, TAXIWAY),
                (1, 4, TAXIWAY),
                (1, 5, TAXIWAY),
            ],
            &[],
            &[],
        )?;
        let sections = Sections::new(&ground);
        assert_eq!(sections.areas().len(), 1);
        assert_eq!(sections.areas()[0].nodes, [Id::new(0), Id::new(1)]);
        assert_eq!(sections.stretches().len(), 4);
        Ok(())
    }

    #[test]
    fn test_a_runway_is_an_area_out_to_its_holding_positions() -> Result<()> {
        // Runway 0-1-2; a taxiway from 1 south through hold 3 (60 m) to 4.
        let ground = layout(
            &[
                (0.0, 0.0),
                (500.0, 0.0),
                (1000.0, 0.0),
                (500.0, -60.0),
                (500.0, -400.0),
            ],
            &[
                (0, 1, EdgeKind::Runway),
                (1, 2, EdgeKind::Runway),
                (1, 3, TAXIWAY),
                (3, 4, TAXIWAY),
            ],
            &[],
            &[3],
        )?;
        let sections = Sections::new(&ground);
        assert_eq!(sections.areas().len(), 1);
        assert!(sections.areas()[0].runway);
        let legs = sections.legs(
            &ground,
            &[(Id::new(3), false), (Id::new(2), false), (Id::new(1), true)],
            0.0,
        );
        // Up to the hold on the stretch, then the runway.
        assert!((legs[0].to - 340.0).abs() < 1.0, "{legs:?}");
        assert_eq!(legs[1].section, Section::Area(0));
        assert!((legs[1].to - 900.0).abs() < 1.0);
        Ok(())
    }

    #[test]
    fn test_shipped_hubs_have_small_areas_and_long_stretches() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        for ident in ["EDDF", "EDDM", "EDDB"] {
            let airport = world.airport_by_ident(ident.parse()?).context("airport")?;
            let ground = network.airport(airport).context("layout")?;
            let sections = Sections::new(ground);
            let largest = sections
                .areas()
                .iter()
                .filter(|a| !a.runway)
                .map(|a| a.nodes.len())
                .max()
                .context("areas")?;
            let stretched: f64 = sections.stretches().iter().sum();
            let taxiways: f64 = ground
                .edges()
                .iter()
                .filter(|e| matches!(e.kind(), EdgeKind::Taxiway | EdgeKind::Taxilane))
                .map(|e| e.length().value())
                .sum();
            assert!(largest <= 8, "{ident}: an area of {largest} junctions");
            assert!(
                stretched > 0.5 * taxiways,
                "{ident}: {stretched:.0} of {taxiways:.0} m on stretches"
            );
        }
        Ok(())
    }
}
