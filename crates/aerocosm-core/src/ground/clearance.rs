//! Progressive taxi clearances (plan M4 D79, D80): which points of an
//! airport's network each aircraft holds, and whether it may be cleared on.
//!
//! An aircraft is cleared hop by hop along its route. A hop is a node, or a
//! point on a long edge, so that aircraft can follow each other along long
//! taxiways. A point is held by one aircraft at a time, and a node also
//! conflicts with the nodes near it (an aircraft is longer than many
//! edges). While anyone is on an edge, it has their direction: nobody is
//! cleared onto it the other way.

use super::{AirportGround, Edge, Node, spacing::close_points};
use crate::Id;

/// Long edges get a clearance point at least every this many metres.
const SEGMENT: f64 = 100.0;

/// A place an aircraft may be cleared to: a node, or a point on an edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hop {
    /// Index of the point among the network's clearance points.
    pub(crate) point: u32,
    /// The node, when the point is one.
    pub(crate) node: Option<Id<Node>>,
    /// Distance along the aircraft's path, m.
    pub(crate) along: f64,
    /// The edge taken to get here, and whether forwards.
    pub(crate) via: Option<(Id<Edge>, bool)>,
}

impl Hop {
    /// Standing at `node`, at the start of a path.
    pub(crate) fn at(node: Id<Node>) -> Self {
        Self {
            point: node.index(),
            node: Some(node),
            along: 0.0,
            via: None,
        }
    }
}

/// Who holds which clearance points and edges at one airport.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Clearances {
    /// Number of nodes; points from here on lie on edges.
    nodes: u32,
    /// Per point, the other points too close for a second aircraft.
    near: Vec<Vec<u32>>,
    /// Per point, the aircraft holding it.
    holder: Vec<Option<u32>>,
    /// Per edge, the aircraft on it and their direction.
    users: Vec<Vec<(u32, bool)>>,
    /// Per edge, the aircraft cleared to take it soon and their direction.
    intents: Vec<Vec<(u32, bool)>>,
    /// Per edge, the direction recent traffic took it, from -1 (against
    /// its `from`→`to` direction) to 1.
    flow: Vec<f32>,
    /// Per point, whether an aircraft may stop there without blocking a
    /// junction: a point on an edge, or a node with at most two edges and
    /// no junction near it.
    stop: Vec<bool>,
    /// Per node, whether three or more taxiways meet there.
    junction: Vec<bool>,
    /// Per edge, the index of its first point on the edge and how many.
    on_edge: Vec<(u32, u32)>,
}

impl Clearances {
    /// The clearance points of `layout`, with `near` per node (the nodes
    /// within an aircraft's length) and every point on an edge near the
    /// points within `radius` m, all free.
    pub(crate) fn new(layout: &AirportGround, near: &[Vec<Id<Node>>], radius: f64) -> Self {
        let nodes = u32::try_from(layout.nodes().len()).unwrap_or(u32::MAX);
        let mut on_edge = Vec::with_capacity(layout.edges().len());
        let mut next = nodes;
        for edge in layout.edges() {
            let count = interior_points(edge.length().value());
            on_edge.push((next, count));
            next = next.saturating_add(count);
        }
        let mut lists: Vec<Vec<u32>> = near
            .iter()
            .map(|list| list.iter().map(|n| n.index()).collect())
            .collect();
        let junction = |n: u32| {
            Id::<Node>::from_usize(n as usize).is_ok_and(|id| layout.edges_at(id).len() >= 3)
        };
        let junctions: Vec<bool> = (0..nodes).map(junction).collect();
        let mut stop = vec![true; next as usize];
        for (point, slot) in stop.iter_mut().enumerate().take(nodes as usize) {
            let point = u32::try_from(point).unwrap_or(u32::MAX);
            *slot = !junction(point)
                && !lists
                    .get(point as usize)
                    .is_some_and(|near| near.iter().any(|&n| junction(n)));
        }
        lists.resize(next as usize, Vec::new());
        // Points on edges against everything near them: parallel lanes and
        // crossings without a node lie closer than an aircraft's length.
        let mut xy: Vec<[f64; 2]> = layout.nodes().iter().map(|node| node.xy).collect();
        xy.reserve((next - nodes) as usize);
        for edge in layout.edges() {
            let count = interior_points(edge.length().value());
            for k in 1..=count {
                let along = edge.length().value() * f64::from(k) / f64::from(count + 1);
                xy.push(layout.edge_point(edge, along));
            }
        }
        for (point, close) in close_points(&xy, radius).into_iter().enumerate() {
            let on_edge = point >= nodes as usize;
            for other in close {
                if on_edge || other >= nodes {
                    lists[point].push(other);
                }
            }
        }
        Self {
            nodes,
            near: lists,
            holder: vec![None; next as usize],
            users: vec![Vec::new(); layout.edges().len()],
            intents: vec![Vec::new(); layout.edges().len()],
            flow: vec![0.0; layout.edges().len()],
            stop,
            junction: junctions,
            on_edge,
        }
    }

    /// The hops of a route from `start` along `edges`, the first at `offset`
    /// metres along the aircraft's path.
    pub(crate) fn hops(
        &self,
        layout: &AirportGround,
        start: Id<Node>,
        edges: &[(Id<Edge>, bool)],
        offset: f64,
    ) -> Vec<Hop> {
        let mut hops = Vec::with_capacity(edges.len() * 2 + 1);
        hops.push(Hop {
            point: start.index(),
            node: Some(start),
            along: offset,
            via: None,
        });
        let mut along = offset;
        for &(id, forward) in edges {
            let Some(edge) = layout.edge(id) else {
                continue;
            };
            let length = edge.length().value();
            let (first, count) = self.on_edge.get(id.as_usize()).copied().unwrap_or((0, 0));
            for k in 1..=count {
                let index = if forward { k } else { count + 1 - k };
                hops.push(Hop {
                    point: first + index - 1,
                    node: None,
                    along: along + length * f64::from(k) / f64::from(count + 1),
                    via: Some((id, forward)),
                });
            }
            along += length;
            let end = if forward { edge.to() } else { edge.from() };
            hops.push(Hop {
                point: end.index(),
                node: Some(end),
                along,
                via: Some((id, forward)),
            });
        }
        hops
    }

    /// Who other than `owner` holds `point` or a point near it.
    fn holder_near(&self, point: u32, owner: u32) -> Option<u32> {
        std::iter::once(point)
            .chain(self.near.get(point as usize).into_iter().flatten().copied())
            .filter_map(|p| self.holder.get(p as usize).copied().flatten())
            .find(|&holder| holder != owner)
    }

    /// Whether points `a` and `b` are the same or too close for two
    /// aircraft.
    pub(crate) fn close(&self, a: u32, b: u32) -> bool {
        a == b
            || self
                .near
                .get(a as usize)
                .is_some_and(|near| near.contains(&b))
    }

    /// Whether three or more taxiways meet at `point`.
    pub(crate) fn is_junction(&self, point: u32) -> bool {
        self.junction.get(point as usize).copied().unwrap_or(false)
    }

    /// Whether an aircraft may stop at `point` without blocking a junction.
    pub(crate) fn stoppable(&self, point: u32) -> bool {
        self.stop.get(point as usize).copied().unwrap_or(true)
    }

    /// Who other than `owner` is on `edge`, or cleared to take it soon,
    /// against direction `forward`.
    pub(crate) fn opposed(&self, owner: u32, edge: Id<Edge>, forward: bool) -> Option<u32> {
        self.against(edge, forward, owner).or_else(|| {
            self.intents
                .get(edge.as_usize())
                .into_iter()
                .flatten()
                .find(|&&(user, way)| user != owner && way != forward)
                .map(|&(user, _)| user)
        })
    }

    /// `owner` is cleared to take `edge` soon, in direction `forward`.
    pub(crate) fn intend(&mut self, owner: u32, edge: Id<Edge>, forward: bool) {
        if let Some(intents) = self.intents.get_mut(edge.as_usize())
            && !intents.iter().any(|&(user, _)| user == owner)
        {
            intents.push((owner, forward));
        }
    }

    /// `owner` no longer intends to take `edge`.
    pub(crate) fn forget(&mut self, owner: u32, edge: Id<Edge>) {
        if let Some(intents) = self.intents.get_mut(edge.as_usize()) {
            intents.retain(|&(user, _)| user != owner);
        }
    }

    /// Who other than `owner` is on `edge` against direction `forward`.
    fn against(&self, edge: Id<Edge>, forward: bool, owner: u32) -> Option<u32> {
        self.users
            .get(edge.as_usize())
            .into_iter()
            .flatten()
            .find(|&&(user, way)| user != owner && way != forward)
            .map(|&(user, _)| user)
    }

    /// Who keeps `owner` from being cleared to `hop`, if anyone: someone at
    /// or near its point, someone on its edge the other way, or someone on
    /// the edge after it (`then`) coming the other way, who would meet it
    /// head-on.
    pub(crate) fn blocker(&self, owner: u32, hop: &Hop, then: Option<&Hop>) -> Option<u32> {
        self.holder_near(hop.point, owner)
            .or_else(|| hop.via.and_then(|(e, f)| self.against(e, f, owner)))
            .or_else(|| {
                then.and_then(|h| h.via)
                    .filter(|&(e, _)| hop.via.is_none_or(|(own, _)| own != e))
                    .and_then(|(e, f)| self.against(e, f, owner))
            })
    }

    /// Clears `owner` to `hop`: it holds the point, unless someone else
    /// does (an aircraft cleared regardless), and is on its edge.
    pub(crate) fn enter(&mut self, owner: u32, hop: &Hop) {
        if let Some(slot) = self.holder.get_mut(hop.point as usize)
            && slot.is_none()
        {
            *slot = Some(owner);
        }
        if let Some((edge, forward)) = hop.via
            && let Some(users) = self.users.get_mut(edge.as_usize())
            && !users.iter().any(|&(user, _)| user == owner)
        {
            users.push((owner, forward));
            if let Some(flow) = self.flow.get_mut(edge.as_usize()) {
                let way = if forward { 1.0 } else { -1.0 };
                *flow = 0.9 * *flow + 0.1 * way;
            }
        }
    }

    /// How much traffic has recently taken `edge` the other way than
    /// `forward`, from 0 (none) to 1 (all).
    pub(crate) fn against_flow(&self, edge: Id<Edge>, forward: bool) -> f64 {
        let flow = f64::from(self.flow.get(edge.as_usize()).copied().unwrap_or(0.0));
        if forward {
            (-flow).max(0.0)
        } else {
            flow.max(0.0)
        }
    }

    /// `owner` has passed `hop`: it frees the point and, at the end of the
    /// edge, leaves the edge.
    pub(crate) fn leave(&mut self, owner: u32, hop: &Hop) {
        if let Some(slot) = self.holder.get_mut(hop.point as usize)
            && *slot == Some(owner)
        {
            *slot = None;
        }
        if hop.node.is_some()
            && let Some((edge, _)) = hop.via
            && let Some(users) = self.users.get_mut(edge.as_usize())
        {
            users.retain(|&(user, _)| user != owner);
        }
    }

    /// Frees everything `owner` holds.
    pub(crate) fn release(&mut self, owner: u32) {
        for slot in &mut self.holder {
            if *slot == Some(owner) {
                *slot = None;
            }
        }
        for users in self.users.iter_mut().chain(&mut self.intents) {
            users.retain(|&(user, _)| user != owner);
        }
    }

    /// Whether `point` is a node rather than a point on an edge.
    #[cfg(test)]
    fn is_node(&self, point: u32) -> bool {
        point < self.nodes
    }
}

/// How many clearance points an edge of `length` metres gets between its
/// nodes.
fn interior_points(length: f64) -> u32 {
    // SAFETY: airport edges are at most a few kilometres.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let segments = (length / SEGMENT).ceil().max(1.0) as u32;
    segments - 1
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        ground::{AirportGroundDefinition, EdgeDefinition, EdgeKind, definition::Point},
        test_data::data,
    };

    /// A `size` × `size` grid of two-way taxiways `step` degrees of latitude
    /// apart near Frankfurt; node `i` at column `i % size`, row `i / size`.
    fn grid(size: u32, step: f64) -> Result<AirportGround> {
        let mut nodes: Vec<Point> = Vec::new();
        let mut edges = Vec::new();
        for row in 0..size {
            for column in 0..size {
                nodes.push([
                    50.03 + f64::from(row) * step,
                    8.56 + f64::from(column) * step * 1.556,
                ]);
                let index = row * size + column;
                let link = |to: u32| EdgeDefinition {
                    from: index,
                    to,
                    kind: EdgeKind::Taxiway,
                    name: None,
                    oneway: false,
                    max_wingspan: None,
                    points: Vec::new(),
                };
                if column + 1 < size {
                    edges.push(link(index + 1));
                }
                if row + 1 < size {
                    edges.push(link(index + size));
                }
            }
        }
        AirportGround::new(
            AirportGroundDefinition {
                airport: "EDDF".parse()?,
                nodes,
                edges,
                stands: Vec::new(),
                holding_positions: Vec::new(),
                exits: Vec::new(),
                intermediate_holds: Vec::new(),
                aprons: Vec::new(),
            },
            &data()?.world,
        )
    }

    /// Two parallel two-way taxiways ~250 m long and 30 m apart, 0-1 and
    /// 2-3, joined at the west end by 0-2.
    fn lanes() -> Result<AirportGround> {
        let link = |from: u32, to: u32| EdgeDefinition {
            from,
            to,
            kind: EdgeKind::Taxiway,
            name: None,
            oneway: false,
            max_wingspan: None,
            points: Vec::new(),
        };
        AirportGround::new(
            AirportGroundDefinition {
                airport: "EDDF".parse()?,
                nodes: vec![
                    [50.03, 8.56],
                    [50.03, 8.5635],
                    [50.03027, 8.56],
                    [50.03027, 8.5635],
                ],
                edges: vec![link(0, 1), link(2, 3), link(0, 2)],
                stands: Vec::new(),
                holding_positions: Vec::new(),
                exits: Vec::new(),
                intermediate_holds: Vec::new(),
                aprons: Vec::new(),
            },
            &data()?.world,
        )
    }

    /// The edge from node `a` to node `b`, and whether it runs that way.
    fn edge(layout: &AirportGround, a: u32, b: u32) -> Result<(Id<Edge>, bool)> {
        layout
            .edges_at(Id::new(a))
            .iter()
            .find_map(|&id| {
                let e = layout.edge(id)?;
                (e.from().index() == a && e.to().index() == b)
                    .then_some((id, true))
                    .or_else(|| {
                        (e.from().index() == b && e.to().index() == a).then_some((id, false))
                    })
            })
            .context("no such edge")
    }

    #[test]
    fn test_hops_follow_the_route_with_points_on_long_edges() -> Result<()> {
        // ~250 m apart: two points on each edge.
        let layout = grid(3, 0.00225)?;
        let clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let route = [edge(&layout, 0, 1)?, edge(&layout, 1, 4)?];
        let hops = clearances.hops(&layout, Id::new(0), &route, 10.0);
        let nodes: Vec<Option<u32>> = hops.iter().map(|h| h.node.map(Id::index)).collect();
        assert_eq!(nodes, [Some(0), None, None, Some(1), None, None, Some(4)]);
        assert!((hops[0].along - 10.0).abs() < 1e-9);
        assert!(hops.windows(2).all(|w| w[1].along > w[0].along));
        let total: f64 = route
            .iter()
            .filter_map(|&(e, _)| layout.edge(e))
            .map(|e| e.length().value())
            .sum();
        assert!((hops[6].along - 10.0 - total).abs() < 1e-6);
        assert!(
            hops.iter()
                .all(|h| h.node.is_some() == clearances.is_node(h.point))
        );
        // Taxied the other way, the same edge's points come in reverse.
        let back = clearances.hops(&layout, Id::new(1), &[edge(&layout, 1, 0)?], 0.0);
        assert_eq!(back[1].point, hops[2].point);
        assert_eq!(back[2].point, hops[1].point);
        Ok(())
    }

    #[test]
    fn test_points_are_held_by_one_aircraft_and_their_neighbours_too() -> Result<()> {
        let layout = grid(3, 0.0008)?;
        // Nodes 0 and 1 count as near each other, as if 40 m apart.
        let mut near = vec![Vec::new(); layout.nodes().len()];
        near[0].push(Id::new(1));
        near[1].push(Id::new(0));
        let mut clearances = Clearances::new(&layout, &near, 45.0);
        let a = clearances.hops(&layout, Id::new(3), &[edge(&layout, 3, 0)?], 0.0);
        clearances.enter(7, &a[1]);
        assert_eq!(clearances.blocker(8, &a[1], None), Some(7));
        // Someone cleared regardless passes without taking the point over.
        clearances.enter(9, &a[1]);
        clearances.leave(9, &a[1]);
        assert_eq!(clearances.blocker(8, &a[1], None), Some(7));
        assert_eq!(clearances.blocker(7, &a[1], None), None);
        // Node 1 is near node 0.
        let b = clearances.hops(&layout, Id::new(2), &[edge(&layout, 2, 1)?], 0.0);
        assert_eq!(clearances.blocker(8, &b[1], None), Some(7));
        clearances.leave(7, &a[1]);
        assert_eq!(clearances.blocker(8, &b[1], None), None);
        assert!(clearances.close(0, 1));
        assert!(clearances.close(1, 1));
        assert!(!clearances.close(0, 2));
        Ok(())
    }

    #[test]
    fn test_points_on_parallel_taxiways_close_together_conflict() -> Result<()> {
        let layout = lanes()?;
        let mut clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let south = clearances.hops(&layout, Id::new(0), &[edge(&layout, 0, 1)?], 0.0);
        let north = clearances.hops(&layout, Id::new(2), &[edge(&layout, 2, 3)?], 0.0);
        assert_eq!(south.len(), north.len());
        assert!(south.len() > 2, "points on the long edges");
        clearances.enter(7, &south[1]);
        assert_eq!(clearances.blocker(8, &north[1], None), Some(7));
        assert_eq!(clearances.blocker(8, &north[2], None), None);
        clearances.leave(7, &south[1]);
        assert_eq!(clearances.blocker(8, &north[1], None), None);
        // Points on an edge stay apart from its own ends: aircraft follow
        // each other along it.
        clearances.enter(7, &south[0]);
        assert_eq!(clearances.blocker(8, &south[1], None), None);
        Ok(())
    }

    #[test]
    fn test_aircraft_stop_only_clear_of_junctions() -> Result<()> {
        // 3 × 3 grid ~250 m apart: the centre and edge midpoints are
        // junctions; the corners have two edges and nothing near.
        let layout = grid(3, 0.00225)?;
        let clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        assert!(clearances.stoppable(0));
        assert!(!clearances.stoppable(4));
        assert!(!clearances.stoppable(1));
        // Points on long edges are always fine.
        let hops = clearances.hops(&layout, Id::new(0), &[edge(&layout, 0, 1)?], 0.0);
        assert!(clearances.stoppable(hops[1].point));
        // Next to a junction is not.
        let mut near = layout.close_nodes(45.0);
        near[0].push(Id::new(1));
        near[1].push(Id::new(0));
        assert!(!Clearances::new(&layout, &near, 45.0).stoppable(0));
        assert!(!clearances.is_junction(0));
        assert!(clearances.is_junction(1));
        assert!(clearances.is_junction(4));
        Ok(())
    }

    #[test]
    fn test_hop_at_stands_at_a_node() -> Result<()> {
        let layout = grid(3, 0.00225)?;
        let mut clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let hop = Hop::at(Id::new(4));
        assert_eq!(hop.node, Some(Id::new(4)));
        assert_eq!(hop.via, None);
        assert!(hop.along.abs() < 1e-9);
        // Holding it holds the node as reached along any route.
        clearances.enter(7, &hop);
        let route = clearances.hops(&layout, Id::new(1), &[edge(&layout, 1, 4)?], 0.0);
        let last = route.last().context("a hop")?;
        assert_eq!(clearances.blocker(8, last, None), Some(7));
        Ok(())
    }

    #[test]
    fn test_intents_keep_opposing_traffic_off_a_route_ahead() -> Result<()> {
        let layout = grid(3, 0.00225)?;
        let mut clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let (e, forward) = edge(&layout, 0, 1)?;
        clearances.intend(7, e, forward);
        // Opposite: refused; same way: fine; its own: fine.
        assert_eq!(clearances.opposed(8, e, !forward), Some(7));
        assert_eq!(clearances.opposed(8, e, forward), None);
        assert_eq!(clearances.opposed(7, e, !forward), None);
        clearances.forget(7, e);
        assert_eq!(clearances.opposed(8, e, !forward), None);
        // Someone on the edge counts too.
        let hops = clearances.hops(&layout, Id::new(0), &[(e, forward)], 0.0);
        clearances.enter(7, &hops[1]);
        assert_eq!(clearances.opposed(8, e, !forward), Some(7));
        clearances.intend(7, e, forward);
        clearances.release(7);
        assert_eq!(clearances.opposed(8, e, !forward), None);
        Ok(())
    }

    #[test]
    fn test_traffic_teaches_each_edge_a_direction() -> Result<()> {
        let layout = grid(3, 0.0008)?;
        let mut clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let (e, forward) = edge(&layout, 0, 1)?;
        // No traffic yet: either way costs nothing extra.
        assert!(clearances.against_flow(e, forward).abs() < 1e-9);
        for owner in 0..20 {
            let hops = clearances.hops(&layout, Id::new(0), &[(e, forward)], 0.0);
            clearances.enter(owner, &hops[1]);
            clearances.release(owner);
        }
        // Against the learnt flow costs more; with it, nothing.
        assert!(clearances.against_flow(e, !forward) > 0.5);
        assert!(clearances.against_flow(e, forward).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_edges_carry_one_direction_and_head_on_meetings_are_refused() -> Result<()> {
        let layout = grid(3, 0.00225)?;
        let mut clearances = Clearances::new(&layout, &layout.close_nodes(45.0), 45.0);
        let east = clearances.hops(&layout, Id::new(0), &[edge(&layout, 0, 1)?], 0.0);
        // Aircraft 7 on its way from node 0 to node 1, at the first point.
        clearances.enter(7, &east[0]);
        clearances.enter(7, &east[1]);
        // Following it the same way is fine once its point is free…
        clearances.leave(7, &east[0]);
        assert_eq!(clearances.blocker(8, &east[0], Some(&east[1])), None);
        // …coming the other way is not, even far from it.
        let west = clearances.hops(&layout, Id::new(1), &[edge(&layout, 1, 0)?], 0.0);
        assert_eq!(clearances.blocker(8, &west[1], None), Some(7));
        // Nor is entering node 1 to take that edge next.
        let down = clearances.hops(
            &layout,
            Id::new(4),
            &[edge(&layout, 4, 1)?, edge(&layout, 1, 0)?],
            0.0,
        );
        let at_one = down
            .iter()
            .position(|h| h.node == Some(Id::new(1)))
            .context("node 1")?;
        assert_eq!(
            clearances.blocker(8, &down[at_one], down.get(at_one + 1)),
            Some(7)
        );
        // Leaving at the edge's end frees it.
        clearances.leave(7, &east[1]);
        clearances.enter(7, &east[3]);
        clearances.leave(7, &east[3]);
        assert_eq!(clearances.blocker(8, &west[1], None), None);
        clearances.enter(7, &east[1]);
        clearances.release(7);
        assert_eq!(clearances.blocker(8, &west[1], None), None);
        Ok(())
    }
}
