//! Junctions and who gives way at them (plan M4 D89, D93).
//!
//! A *junction* is one physical intersection: the junction nodes OSM draws
//! around its fillets, clustered. A runway is a junction too, entered only
//! from its holding positions. The taxiways between junctions are *chains*.
//!
//! At each junction the approaches with a holding position yield: a mapped
//! intermediate or runway hold, else one synthesised on the approaches
//! that are not the through taxiway (the pair with the same name, else the
//! straightest). The through taxiway has right of way.

use super::{AirportGround, Edge, EdgeKind, Node};
use crate::Id;

/// Junction nodes this close along a chain are one intersection, m.
const CLUSTER: f64 = 30.0;
/// A synthesised hold this far from its junction, m: a waiting aircraft's
/// nose keeps its wings clear of the crossing taxiway.
const YIELD_HOLD: f64 = 45.0;
/// A synthesised runway hold this far from the runway, m.
const RUNWAY_HOLD: f64 = 90.0;
/// Approaches turning less than this off straight on are a through pair
/// without a common name, degrees.
const STRAIGHT: f64 = 30.0;

/// One intersection or runway.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Junction {
    /// Its nodes.
    pub(crate) nodes: Vec<Id<Node>>,
    /// Whether it is a runway.
    pub(crate) runway: bool,
}

/// A junction a route passes: where it enters and leaves the junction's
/// nodes, metres along the aircraft's path, and where it holds short if
/// its approach gives way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Passage {
    pub(crate) junction: u32,
    pub(crate) entry: f64,
    pub(crate) exit: f64,
    pub(crate) hold: Option<f64>,
}

/// A chain a route runs along: from `from` to `to` metres along the
/// aircraft's path, entering at `at` metres along the chain, and whether
/// the chain's own way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Run {
    pub(crate) chain: u32,
    pub(crate) from: f64,
    pub(crate) to: f64,
    pub(crate) at: f64,
    pub(crate) forward: bool,
}

impl Run {
    /// Where on the chain an aircraft `along` metres along its path is.
    pub(crate) fn position(&self, along: f64) -> f64 {
        if self.forward {
            self.at + (along - self.from)
        } else {
            self.at - (along - self.from)
        }
    }

    /// How far along the path the chain's `position` is.
    pub(crate) fn along(&self, position: f64) -> f64 {
        if self.forward {
            self.from + (position - self.at)
        } else {
            self.from + (self.at - position)
        }
    }
}

/// An airport's junctions, chains and holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Junctions {
    junctions: Vec<Junction>,
    /// Per node, its junction.
    junction_of: Vec<Option<u32>>,
    /// Per edge, its chain, whether it runs the chain's way and where its
    /// first node in the chain's way lies along the chain, m; lead-ins and
    /// edges inside a junction have none.
    chain_of: Vec<Option<(u32, bool, f64)>>,
    /// Per chain, its length, m.
    chains: Vec<f64>,
    /// Per edge and end (`from`, `to`): the hold of an approach that gives
    /// way at the junction there, m before reaching it.
    holds: Vec<[Option<f64>; 2]>,
}

/// A chain of edges between junctions or dead ends.
struct Chain {
    edges: Vec<(Id<Edge>, bool)>,
    length: f64,
    start: Option<Id<Node>>,
    end: Option<Id<Node>>,
    /// Distances of hold nodes along it.
    holds: Vec<f64>,
    /// Bearing leaving the start and leaving the end, degrees.
    bearings: [f64; 2],
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

fn usable(layout: &AirportGround, edge: Id<Edge>) -> bool {
    layout
        .edge(edge)
        .is_some_and(|e| matches!(e.kind(), EdgeKind::Taxiway | EdgeKind::Taxilane))
}

/// Bearing of a polyline's first piece, degrees.
fn bearing(from: [f64; 2], to: [f64; 2]) -> f64 {
    (to[0] - from[0])
        .atan2(to[1] - from[1])
        .to_degrees()
        .rem_euclid(360.0)
}

impl Junctions {
    /// The junctions of `layout`.
    pub(crate) fn new(layout: &AirportGround) -> Self {
        let count = layout.nodes().len();
        let degree = |n: usize| {
            Id::<Node>::from_usize(n).map_or(0, |id| {
                layout
                    .edges_at(id)
                    .iter()
                    .filter(|&&e| usable(layout, e))
                    .count()
            })
        };
        let on_runway: Vec<bool> = (0..count)
            .map(|n| {
                Id::<Node>::from_usize(n).is_ok_and(|id| {
                    layout
                        .edges_at(id)
                        .iter()
                        .any(|&e| layout.edge(e).is_some_and(|e| e.kind() == EdgeKind::Runway))
                })
            })
            .collect();
        let special: Vec<bool> = (0..count).map(|n| on_runway[n] || degree(n) >= 3).collect();
        let mut hold = vec![false; count];
        for &node in layout.intermediate_holds() {
            hold[node.as_usize()] = true;
        }
        for position in layout.holding_positions() {
            hold[position.node().as_usize()] = true;
        }
        let chains = chains(layout, &special, &hold);
        let mut merge = Merge((0..count).collect());
        for edge in layout.edges() {
            if edge.kind() == EdgeKind::Runway {
                merge.join(edge.from().as_usize(), edge.to().as_usize());
            }
        }
        // Junction nodes close together are one intersection; a runway
        // keeps its own.
        let mut inner = vec![false; chains.len()];
        for (i, chain) in chains.iter().enumerate() {
            if let (Some(a), Some(b)) = (chain.start, chain.end)
                && chain.length < CLUSTER
                && on_runway[a.as_usize()] == on_runway[b.as_usize()]
            {
                merge.join(a.as_usize(), b.as_usize());
                inner[i] = true;
            }
        }
        let mut junction_of = vec![None; count];
        let mut index_of_root = vec![None; count];
        let mut junctions: Vec<Junction> = Vec::new();
        for node in 0..count {
            if !special[node] {
                continue;
            }
            let root = merge.root(node);
            let index = *index_of_root[root].get_or_insert_with(|| {
                junctions.push(Junction {
                    nodes: Vec::new(),
                    runway: false,
                });
                u32::try_from(junctions.len() - 1).unwrap_or(u32::MAX)
            });
            junction_of[node] = Some(index);
            if let (Some(junction), Ok(id)) =
                (junctions.get_mut(index as usize), Id::from_usize(node))
            {
                junction.runway |= on_runway[node];
                junction.nodes.push(id);
            }
        }
        // Approaches per junction: (chain, end 0 = start / 1 = end).
        let mut approaches: Vec<Vec<(usize, usize)>> = vec![Vec::new(); junctions.len()];
        for (i, chain) in chains.iter().enumerate() {
            if inner[i] {
                continue;
            }
            for (end, node) in [(0, chain.start), (1, chain.end)] {
                if let Some(j) = node.and_then(|n| junction_of[n.as_usize()]) {
                    approaches[j as usize].push((i, end));
                }
            }
        }
        // Where each approach holds, m before its junction.
        let mut hold_at: Vec<[Option<f64>; 2]> = vec![[None, None]; chains.len()];
        for (j, list) in approaches.iter().enumerate() {
            let runway = junctions[j].runway;
            let through = if runway {
                Vec::new()
            } else {
                through_pair(layout, &chains, list)
            };
            for &(i, end) in list {
                let chain = &chains[i];
                let near = |d: f64| if end == 0 { d } else { chain.length - d };
                let real = chain
                    .holds
                    .iter()
                    .map(|&d| near(d))
                    .filter(|&d| d <= chain.length / 2.0)
                    .fold(f64::INFINITY, f64::min);
                hold_at[i][end] = if real.is_finite() {
                    Some(real)
                } else if runway {
                    Some(RUNWAY_HOLD.min(chain.length))
                } else if through.contains(&(i, end)) {
                    None
                } else {
                    Some(YIELD_HOLD.min(chain.length / 2.0))
                };
            }
        }
        let mut chain_of = vec![None; layout.edges().len()];
        let mut holds = vec![[None, None]; layout.edges().len()];
        let mut lengths = Vec::new();
        for (i, chain) in chains.iter().enumerate() {
            if inner[i] {
                continue;
            }
            let id = u32::try_from(lengths.len()).unwrap_or(u32::MAX);
            lengths.push(chain.length);
            let mut walked = 0.0;
            for &(edge, forward) in &chain.edges {
                chain_of[edge.as_usize()] = Some((id, forward, walked));
                walked += layout.edge(edge).map_or(0.0, |e| e.length().value());
            }
            // A hold is kept on the edge that reaches its junction.
            if let (Some(&(first, forward)), Some(h)) = (chain.edges.first(), hold_at[i][0]) {
                holds[first.as_usize()][usize::from(!forward)] = Some(h);
            }
            if let (Some(&(last, forward)), Some(h)) = (chain.edges.last(), hold_at[i][1]) {
                holds[last.as_usize()][usize::from(forward)] = Some(h);
            }
        }
        Self {
            junctions,
            junction_of,
            chain_of,
            chains: lengths,
            holds,
        }
    }

    /// The junctions.
    pub(crate) fn junctions(&self) -> &[Junction] {
        &self.junctions
    }

    /// Each chain's length, m.
    pub(crate) fn chains(&self) -> &[f64] {
        &self.chains
    }

    /// The junction `node` belongs to, if any.
    pub(crate) fn junction_of(&self, node: Id<Node>) -> Option<u32> {
        self.junction_of.get(node.as_usize()).copied().flatten()
    }

    /// The junctions a route from `start` along `edges` passes, its first
    /// node `offset` metres along the aircraft's path, and the chains it
    /// runs along.
    pub(crate) fn passages(
        &self,
        layout: &AirportGround,
        start: Id<Node>,
        edges: &[(Id<Edge>, bool)],
        offset: f64,
    ) -> (Vec<Passage>, Vec<Run>) {
        let mut passages: Vec<Passage> = Vec::with_capacity(edges.len() / 2 + 1);
        let mut runs: Vec<Run> = Vec::with_capacity(edges.len() / 2 + 1);
        let mut along = offset;
        let visit =
            |junction: Option<u32>, at: f64, hold: Option<f64>, passages: &mut Vec<Passage>| {
                let Some(junction) = junction else {
                    return;
                };
                match passages.last_mut() {
                    Some(last) if last.junction == junction && (at - last.exit) < CLUSTER * 2.0 => {
                        last.exit = at;
                    }
                    _ => passages.push(Passage {
                        junction,
                        entry: at,
                        exit: at,
                        hold: hold.map(|h| at - h),
                    }),
                }
            };
        visit(self.junction_of(start), along, None, &mut passages);
        for &(id, forward) in edges {
            let Some(edge) = layout.edge(id) else {
                continue;
            };
            let length = edge.length().value();
            if let Some((chain, way, start)) = self.chain_of.get(id.as_usize()).copied().flatten() {
                let same = way == forward;
                match runs.last_mut() {
                    Some(last) if last.chain == chain && (last.to - along).abs() < 1e-6 => {
                        last.to = along + length;
                    }
                    _ => runs.push(Run {
                        chain,
                        from: along,
                        to: along + length,
                        at: if same { start } else { start + length },
                        forward: same,
                    }),
                }
            }
            along += length;
            let end = if forward { edge.to() } else { edge.from() };
            let hold = self
                .holds
                .get(id.as_usize())
                .and_then(|h| h[usize::from(forward)]);
            visit(self.junction_of(end), along, hold, &mut passages);
        }
        (passages, runs)
    }
}

/// Of the approaches `list` to one junction, the through pair: the two with
/// the same taxiway name, else the straightest within `STRAIGHT`; none if
/// neither.
fn through_pair(
    layout: &AirportGround,
    chains: &[Chain],
    list: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    let name = |&(i, end): &(usize, usize)| {
        let chain: &Chain = &chains[i];
        let edge = if end == 0 {
            chain.edges.first()
        } else {
            chain.edges.last()
        };
        edge.and_then(|&(e, _)| layout.edge(e)).and_then(Edge::name)
    };
    let lane = |&(i, _): &(usize, usize)| {
        chains[i].edges.iter().any(|&(e, _)| {
            layout
                .edge(e)
                .is_some_and(|e| e.kind() == EdgeKind::Taxilane)
        })
    };
    let leaving = |&(i, end): &(usize, usize)| chains[i].bearings[end];
    let mut best: Option<((u8, f64), [(usize, usize); 2])> = None;
    for (k, a) in list.iter().enumerate() {
        for b in &list[k + 1..] {
            let turn = (leaving(a) - leaving(b) + 540.0).rem_euclid(360.0) - 180.0;
            let off_straight = 180.0 - turn.abs();
            let named = name(a).is_some() && name(a) == name(b);
            if !named && off_straight > STRAIGHT {
                continue;
            }
            // Same name first, then taxiways before lanes, then straighter.
            let rank = (
                u8::from(!named) * 2 + u8::from(lane(a) || lane(b)),
                off_straight,
            );
            if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, [*a, *b]));
            }
        }
    }
    best.map_or_else(Vec::new, |(_, pair)| pair.to_vec())
}

/// The chains of `layout`'s taxiways and taxilanes between `special` nodes
/// or dead ends, each walked once, with the `hold` nodes along them.
fn chains(layout: &AirportGround, special: &[bool], hold: &[bool]) -> Vec<Chain> {
    let degree = |n: Id<Node>| {
        layout
            .edges_at(n)
            .iter()
            .filter(|&&e| usable(layout, e))
            .count()
    };
    let xy = |n: Id<Node>| layout.nodes()[n.as_usize()].xy;
    // The bearing leaving `node` along `edge`, by its first shape point.
    let leaving = |edge: &Edge, node: Id<Node>| {
        let points = edge.points();
        let next = if edge.from() == node {
            points.get(1)
        } else {
            points.len().checked_sub(2).and_then(|i| points.get(i))
        };
        let to = next.map_or_else(
            || {
                xy(if edge.from() == node {
                    edge.to()
                } else {
                    edge.from()
                })
            },
            |p| layout.local(*p),
        );
        bearing(xy(node), to)
    };
    let mut seen = vec![false; layout.edges().len()];
    let mut chains = Vec::new();
    let mut starts: Vec<Id<Node>> = (0..layout.nodes().len())
        .filter_map(|n| Id::from_usize(n).ok())
        .filter(|&n| special[n.as_usize()] || degree(n) == 1)
        .collect();
    starts.extend((0..layout.nodes().len()).filter_map(|n| Id::from_usize(n).ok()));
    for start in starts {
        for &first in layout.edges_at(start) {
            if seen[first.as_usize()] || !usable(layout, first) {
                continue;
            }
            let Some(first_edge) = layout.edge(first) else {
                continue;
            };
            let mut chain = Chain {
                edges: Vec::new(),
                length: 0.0,
                start: special[start.as_usize()].then_some(start),
                end: None,
                holds: Vec::new(),
                bearings: [leaving(first_edge, start), 0.0],
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
                chain.bearings[1] = leaving(e, node);
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
                    .find(|&e| usable(layout, e) && !seen[e.as_usize()]);
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
mod tests;
