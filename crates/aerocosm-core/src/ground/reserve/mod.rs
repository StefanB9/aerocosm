//! Ground control's reservation table (plan M4 D70): who uses which node
//! and edge of an airport's network when, so taxi plans never conflict.
//!
//! Times are milliseconds of simulation time. A node is held from shortly
//! before an aircraft arrives until shortly after it leaves; an edge records
//! each traversal with its direction. Aircraft on the same edge in the same
//! direction keep a time headway; opposite directions never share an edge.

mod plan;

pub(crate) use self::plan::{PlanRequest, TimedRoute, along_at, plan_route};
use super::{Edge, Node};
use crate::Id;

/// A node is held this long before arrival and after departure, ms.
pub(crate) const NODE_BUFFER: i64 = 15_000;
/// Aircraft following each other along an edge keep this time apart, ms.
pub(crate) const HEADWAY: i64 = 30_000;

/// A node held by one aircraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) from: i64,
    pub(crate) until: i64,
    pub(crate) owner: u32,
    /// The holder waits there for something else (a take-off clearance, a
    /// stand); `until` is only an estimate and may grow.
    pub(crate) open: bool,
}

/// One traversal of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pass {
    pub(crate) enter: i64,
    pub(crate) exit: i64,
    /// From the edge's `from` node to its `to` node.
    pub(crate) forward: bool,
    pub(crate) owner: u32,
}

/// Every node and edge reservation of one airport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Reservations {
    nodes: Vec<Vec<Window>>,
    edges: Vec<Vec<Pass>>,
    /// Per node, the other nodes too close to it for two aircraft.
    near: Vec<Vec<Id<Node>>>,
}

fn overlaps(a: (i64, i64), b: (i64, i64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

impl Reservations {
    /// An empty table for a network of `nodes` nodes and `edges` edges.
    pub(crate) fn new(nodes: usize, edges: usize) -> Self {
        Self {
            nodes: vec![Vec::new(); nodes],
            edges: vec![Vec::new(); edges],
            near: Vec::new(),
        }
    }

    /// The table with `near`, per node, the nodes too close to it for two
    /// aircraft: a window on one holds them all.
    pub(crate) fn with_neighbours(mut self, near: Vec<Vec<Id<Node>>>) -> Self {
        self.near = near;
        self
    }

    /// The windows of others on `node` and the nodes near it.
    fn around(&self, node: Id<Node>, owner: u32) -> impl Iterator<Item = &Window> {
        let near = self
            .near
            .get(node.as_usize())
            .map_or(&[][..], Vec::as_slice);
        std::iter::once(node)
            .chain(near.iter().copied())
            .flat_map(|n| self.windows(n))
            .filter(move |w| w.owner != owner)
    }

    /// The windows on `node`.
    pub(crate) fn windows(&self, node: Id<Node>) -> &[Window] {
        self.nodes.get(node.as_usize()).map_or(&[], Vec::as_slice)
    }

    /// The traversals of `edge`.
    pub(crate) fn passes(&self, edge: Id<Edge>) -> &[Pass] {
        self.edges.get(edge.as_usize()).map_or(&[], Vec::as_slice)
    }

    /// Whether someone other than `owner` holds `node`, or a node near it,
    /// during `[from, until)`.
    pub(crate) fn node_taken(&self, node: Id<Node>, from: i64, until: i64, owner: u32) -> bool {
        self.around(node, owner)
            .any(|w| overlaps((w.from, w.until), (from, until)))
    }

    /// Whether an aircraft may not enter `node` (or a node near it) for
    /// `[from, until)`: like [`Self::node_taken`], except that someone
    /// waiting there holds it until it leaves, whatever its estimate.
    pub(crate) fn node_held(&self, node: Id<Node>, from: i64, until: i64, owner: u32) -> bool {
        self.around(node, owner)
            .any(|w| w.from < until && (w.open || from < w.until))
    }

    /// The end of the latest window of someone other than `owner` on `node`
    /// or a node near it overlapping `[from, until)`, if any.
    pub(crate) fn node_free_after(
        &self,
        node: Id<Node>,
        from: i64,
        until: i64,
        owner: u32,
    ) -> Option<i64> {
        self.around(node, owner)
            .filter(|w| overlaps((w.from, w.until), (from, until)))
            .map(|w| w.until)
            .max()
    }

    /// When a traversal of `edge` entering at `enter` and leaving at `exit`
    /// in direction `forward` would conflict with someone else's, the
    /// earliest entry time that might not; `None` if it is free.
    pub(crate) fn edge_free_after(
        &self,
        edge: Id<Edge>,
        enter: i64,
        exit: i64,
        forward: bool,
        owner: u32,
    ) -> Option<i64> {
        let mut later: Option<i64> = None;
        for pass in self.passes(edge) {
            if pass.owner == owner {
                continue;
            }
            let conflict = if pass.forward == forward {
                // Same direction: keep the headway at both ends, no overtaking.
                (pass.enter - enter).abs() < HEADWAY
                    || (pass.exit - exit).abs() < HEADWAY
                    || (pass.enter < enter) != (pass.exit < exit)
            } else {
                overlaps((pass.enter, pass.exit), (enter, exit))
            };
            if conflict {
                let after = if pass.forward == forward {
                    (pass.enter + HEADWAY).max(enter + (pass.exit + HEADWAY - exit))
                } else {
                    pass.exit
                };
                later = Some(later.map_or(after, |l| l.max(after)));
            }
        }
        later
    }

    /// Records `route` for `owner`.
    pub(crate) fn reserve(&mut self, route: &TimedRoute, owner: u32) {
        for (index, step) in route.steps.iter().enumerate() {
            let from = if index == 0 {
                step.arrive
            } else {
                step.arrive - NODE_BUFFER
            };
            if let Some(windows) = self.nodes.get_mut(step.node.as_usize()) {
                windows.push(Window {
                    from,
                    until: step.leave + NODE_BUFFER,
                    owner,
                    open: route.open && index + 1 == route.steps.len(),
                });
            }
            if let Some((edge, forward)) = step.via
                && let (Some(previous), Some(passes)) = (
                    index.checked_sub(1).and_then(|i| route.steps.get(i)),
                    self.edges.get_mut(edge.as_usize()),
                )
            {
                passes.push(Pass {
                    enter: previous.leave,
                    exit: step.arrive,
                    forward,
                    owner,
                });
            }
        }
    }

    /// Holds `node` for `owner` from `from` until `until` exactly.
    pub(crate) fn occupy(&mut self, node: Id<Node>, from: i64, until: i64, owner: u32) {
        if let Some(windows) = self.nodes.get_mut(node.as_usize()) {
            windows.push(Window {
                from,
                until,
                owner,
                open: false,
            });
        }
    }

    /// Holds `node` for `owner` from `from` until about `until`, open.
    pub(crate) fn hold(&mut self, node: Id<Node>, from: i64, until: i64, owner: u32) {
        if let Some(windows) = self.nodes.get_mut(node.as_usize()) {
            windows.push(Window {
                from,
                until,
                owner,
                open: true,
            });
        }
    }

    /// Keeps every open window running at least until `now + ahead`: their
    /// holders are still waiting.
    pub(crate) fn refresh_open(&mut self, now: i64, ahead: i64) {
        for windows in &mut self.nodes {
            for window in windows.iter_mut().filter(|w| w.open) {
                window.until = window.until.max(now + ahead);
            }
        }
    }

    /// Forgets everything `owner` holds.
    pub(crate) fn release(&mut self, owner: u32) {
        for windows in &mut self.nodes {
            windows.retain(|w| w.owner != owner);
        }
        for passes in &mut self.edges {
            passes.retain(|p| p.owner != owner);
        }
    }

    /// Forgets windows and passes over before `time`, except open windows.
    pub(crate) fn prune(&mut self, time: i64) {
        for windows in &mut self.nodes {
            windows.retain(|w| w.open || w.until > time);
        }
        for passes in &mut self.edges {
            passes.retain(|p| p.exit > time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_windows_conflict_only_with_others() {
        let mut table = Reservations::new(2, 1);
        table.hold(Id::new(0), 1_000, 5_000, 7);
        assert!(table.node_taken(Id::new(0), 4_000, 6_000, 1));
        assert!(!table.node_taken(Id::new(0), 4_000, 6_000, 7));
        assert!(!table.node_taken(Id::new(0), 5_000, 6_000, 1));
        assert_eq!(table.node_free_after(Id::new(0), 0, 2_000, 1), Some(5_000));
        table.refresh_open(10_000, 5_000);
        assert_eq!(table.node_free_after(Id::new(0), 0, 2_000, 1), Some(15_000));
        table.prune(20_000);
        // Open windows survive pruning until released.
        assert_eq!(table.windows(Id::new(0)).len(), 1);
        table.release(7);
        assert_eq!(table.windows(Id::new(0)).len(), 0);
        // A closed window ends when it says.
        table.occupy(Id::new(1), 0, 5_000, 3);
        table.refresh_open(4_000, 5_000);
        assert!(!table.node_taken(Id::new(1), 5_000, 9_000, 1));
        table.prune(6_000);
        assert_eq!(table.windows(Id::new(1)).len(), 0);
        assert_eq!(table.windows(Id::new(9)).len(), 0);
    }

    #[test]
    fn test_open_windows_hold_until_released() {
        let mut table = Reservations::new(1, 0);
        // Waiting there with an estimated end at 20 s.
        table.hold(Id::new(0), 0, 20_000, 7);
        // Planning trusts the estimate…
        assert!(!table.node_taken(Id::new(0), 30_000, 40_000, 1));
        // …but entering the node waits until the holder has gone.
        assert!(table.node_held(Id::new(0), 30_000, 40_000, 1));
        assert!(!table.node_held(Id::new(0), 30_000, 40_000, 7));
        table.release(7);
        assert!(!table.node_held(Id::new(0), 30_000, 40_000, 1));
        // A closed window ends when it says.
        table.occupy(Id::new(0), 0, 20_000, 7);
        assert!(!table.node_held(Id::new(0), 30_000, 40_000, 1));
        assert!(table.node_held(Id::new(0), 10_000, 40_000, 1));
    }

    #[test]
    fn test_nodes_close_together_are_held_together() {
        // Nodes 0 and 1 lie closer than an aircraft is long; node 2 apart.
        let mut table = Reservations::new(3, 0).with_neighbours(vec![
            vec![Id::new(1)],
            vec![Id::new(0)],
            Vec::new(),
        ]);
        table.occupy(Id::new(1), 0, 10_000, 7);
        assert!(table.node_taken(Id::new(0), 5_000, 6_000, 1));
        assert_eq!(table.node_free_after(Id::new(0), 0, 2_000, 1), Some(10_000));
        assert!(!table.node_taken(Id::new(2), 5_000, 6_000, 1));
        // Its own windows never stop the holder.
        assert!(!table.node_taken(Id::new(0), 5_000, 6_000, 7));
    }

    #[test]
    fn test_edges_keep_headway_and_forbid_head_on() {
        let mut table = Reservations::new(2, 1);
        let edge = Id::new(0);
        table.edges[0].push(Pass {
            enter: 0,
            exit: 60_000,
            forward: true,
            owner: 1,
        });
        // Following 40 s behind is fine, 10 s is not.
        assert_eq!(table.edge_free_after(edge, 40_000, 100_000, true, 2), None);
        assert_eq!(
            table.edge_free_after(edge, 10_000, 70_000, true, 2),
            Some(30_000)
        );
        // Meeting it head-on is not, until it has left.
        assert_eq!(
            table.edge_free_after(edge, 30_000, 90_000, false, 2),
            Some(60_000)
        );
        assert_eq!(table.edge_free_after(edge, 60_000, 120_000, false, 2), None);
        table.release(1);
        assert_eq!(table.passes(edge).len(), 0);
    }
}
