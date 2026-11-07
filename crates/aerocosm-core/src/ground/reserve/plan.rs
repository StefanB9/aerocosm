//! The timed planner: the earliest-arriving cheap route from one node to
//! another that conflicts with no reservation, waiting at nodes where needed.

use std::{cmp::Ordering, collections::BinaryHeap};

use super::{NODE_BUFFER, Reservations};
use crate::{
    Id, LatLon,
    ground::{AirportGround, Edge, EdgeKind, Node},
    units::Meters,
};

/// Runway edges cost this many times their duration: backtracking only.
const RUNWAY_PENALTY: f64 = 4.0;
/// Lead-ins of other stands cost this many times their duration.
const LEAD_IN_PENALTY: f64 = 20.0;
/// Attempts to find a conflict-free departure time along one edge.
const ATTEMPTS: usize = 24;
/// A blocked node (an active runway) is tried again this much later, ms.
const BLOCKED_RETRY: i64 = 15_000;

/// What to plan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanRequest<B, C> {
    pub(crate) from: Id<Node>,
    pub(crate) to: Id<Node>,
    /// When the aircraft is at `from`, ready to go, ms.
    pub(crate) start: i64,
    pub(crate) wingspan: Meters,
    /// Taxi speed, m/s.
    pub(crate) speed: f64,
    pub(crate) owner: u32,
    /// How long `to` is held after arrival, ms; an estimate when `open`.
    pub(crate) stay: i64,
    pub(crate) open: bool,
    /// Plans arriving later than this fail, ms.
    pub(crate) deadline: i64,
    /// Longest wait at `from` before moving, ms.
    pub(crate) start_wait: i64,
    /// The aircraft already stands at `from` together with others (after a
    /// hold-up): their windows there do not stop it leaving.
    pub(crate) start_shared: bool,
    /// Whether a node may not be passed during `[from, until)`, e.g. an
    /// active runway in use.
    pub(crate) blocked: B,
    /// Extra cost of an edge, as a multiple of its duration, e.g. along an
    /// active runway: used only where nothing else leads on.
    pub(crate) surcharge: C,
    /// At the goal, bookings of others that start at or after this time do
    /// not count: an aircraft held up long goes first (ms).
    pub(crate) goal_first_from: Option<i64>,
}

/// One node of a timed route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) node: Id<Node>,
    pub(crate) arrive: i64,
    pub(crate) leave: i64,
    /// The edge taken from the previous step, and whether forwards.
    pub(crate) via: Option<(Id<Edge>, bool)>,
}

/// A conflict-free route with its times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimedRoute {
    pub(crate) steps: Vec<Step>,
    /// The last node is held open (waiting for a clearance or a stand).
    pub(crate) open: bool,
}

impl TimedRoute {
    /// Arrival at the last node, ms.
    pub(crate) fn arrival(&self) -> i64 {
        self.steps.last().map_or(0, |step| step.arrive)
    }

    /// The polyline of the route.
    pub(crate) fn points(&self, ground: &AirportGround) -> Vec<LatLon> {
        let mut points = Vec::with_capacity(self.steps.len() * 4);
        if let Some(first) = self.steps.first() {
            points.push(ground.nodes()[first.node.as_usize()].position());
        }
        for step in &self.steps {
            let Some((edge, forward)) = step.via else {
                continue;
            };
            let Some(edge) = ground.edge(edge) else {
                continue;
            };
            if forward {
                points.extend_from_slice(&edge.points()[1..]);
            } else {
                points.extend(edge.points().iter().rev().skip(1));
            }
        }
        points
    }

    /// The polyline from the start to step `last` (inclusive).
    pub(crate) fn points_until(&self, ground: &AirportGround, last: usize) -> Vec<LatLon> {
        let head = TimedRoute {
            steps: self.steps[..=last.min(self.steps.len().saturating_sub(1))].to_vec(),
            open: false,
        };
        head.points(ground)
    }

    /// `(time ms, distance m)` breakpoints: distance along the route is
    /// linear between them.
    pub(crate) fn schedule(&self, ground: &AirportGround) -> Vec<(i64, f64)> {
        let mut breakpoints = Vec::with_capacity(self.steps.len() * 2);
        let mut along = 0.0;
        let last = self.steps.len().saturating_sub(1);
        for (index, step) in self.steps.iter().enumerate() {
            if let Some((edge, _)) = step.via {
                along += ground.edge(edge).map_or(0.0, |e| e.length().value());
            }
            breakpoints.push((step.arrive, along));
            // The stay at the goal is a reservation, not part of the taxi.
            if step.leave > step.arrive && index < last {
                breakpoints.push((step.leave, along));
            }
        }
        breakpoints
    }
}

/// Distance along a schedule at `time`, ms: clamped to its ends.
pub(crate) fn along_at(schedule: &[(i64, f64)], time: i64) -> f64 {
    let index = schedule.partition_point(|(at, _)| *at <= time);
    match (
        index.checked_sub(1).and_then(|i| schedule.get(i)),
        schedule.get(index),
    ) {
        (Some(&(t0, a0)), Some(&(t1, a1))) if t1 > t0 => {
            #[allow(clippy::cast_precision_loss)]
            let fraction = (time - t0) as f64 / (t1 - t0) as f64;
            a0 + (a1 - a0) * fraction
        }
        (Some(&(_, a)), _) | (None, Some(&(_, a))) => a,
        (None, None) => 0.0,
    }
}

/// A search label: reaching `node` at `time` with `cost`.
#[derive(Debug, Clone, Copy)]
struct Label {
    node: usize,
    time: i64,
    cost: f64,
    previous: Option<usize>,
    /// Edge taken from the previous label, direction, and departure time
    /// from the previous node.
    via: Option<(Id<Edge>, bool, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Queued {
    cost: f64,
    label: usize,
}

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.label.cmp(&self.label))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn millis(seconds: f64) -> i64 {
    // SAFETY: taxi durations are minutes at most.
    #[allow(clippy::cast_possible_truncation)]
    let ms = (seconds * 1000.0).ceil() as i64;
    ms.max(1)
}

/// The earliest departure from `here` (reached at `time`) along `edge`
/// that conflicts with nothing; `None` if waiting would itself conflict or
/// the deadline passes.
fn departure<B: Fn(Id<Node>, i64, i64) -> bool, C>(
    table: &Reservations,
    request: &PlanRequest<B, C>,
    here: Id<Node>,
    time: i64,
    edge: (Id<Edge>, bool, Id<Node>),
    duration: i64,
) -> Option<i64> {
    let (edge, forward, next) = edge;
    let stay = if next == request.to { request.stay } else { 0 };
    let mut leave = time;
    for _ in 0..ATTEMPTS {
        let arrive = leave + duration;
        if arrive > request.deadline {
            return None;
        }
        let at_start = here == request.from;
        if at_start && leave - time > request.start_wait {
            return None;
        }
        // Waiting here must not run into someone else's window.
        if !(at_start && request.start_shared)
            && table.node_taken(here, time, leave + NODE_BUFFER, request.owner)
        {
            return None;
        }
        if let Some(later) = table.edge_free_after(edge, leave, arrive, forward, request.owner) {
            leave = later.max(leave + 1);
            continue;
        }
        let (from, until) = (arrive - NODE_BUFFER, arrive + stay + NODE_BUFFER);
        let first_from = request.goal_first_from.filter(|_| next == request.to);
        if let Some(free) = table.node_free_after_from(next, from, until, request.owner, first_from)
        {
            leave = (free + NODE_BUFFER - duration).max(leave + 1);
            continue;
        }
        if (request.blocked)(next, from, until) {
            leave += BLOCKED_RETRY;
            continue;
        }
        return Some(leave);
    }
    None
}

/// Plans `request` around the reservations in `table`; `None` if no route
/// reaches the goal before the deadline.
pub(crate) fn plan_route<B: Fn(Id<Node>, i64, i64) -> bool, C: Fn(Id<Edge>) -> f64>(
    ground: &AirportGround,
    table: &Reservations,
    request: &PlanRequest<B, C>,
) -> Option<TimedRoute> {
    let count = ground.nodes().len();
    let (start, goal) = (request.from.as_usize(), request.to.as_usize());
    if start >= count || goal >= count || request.speed <= 0.0 {
        return None;
    }
    let mut settled = vec![false; count];
    let mut best = vec![f64::INFINITY; count];
    let mut labels = vec![Label {
        node: start,
        time: request.start,
        cost: 0.0,
        previous: None,
        via: None,
    }];
    // Straight-line time to the goal: never more than any route takes.
    let estimate = |node: usize| ground.remaining(node, goal) / request.speed * 1000.0;
    let mut heap = BinaryHeap::with_capacity(64);
    heap.push(Queued {
        cost: estimate(start),
        label: 0,
    });
    best[start] = 0.0;
    let mut found = None;
    while let Some(Queued { label, .. }) = heap.pop() {
        let current = labels[label];
        if settled[current.node] {
            continue;
        }
        settled[current.node] = true;
        if current.node == goal {
            found = Some(label);
            break;
        }
        let here = Id::<Node>::new(u32::try_from(current.node).ok()?);
        for &edge_id in ground.edges_at(here) {
            let Some(edge) = ground.edge(edge_id) else {
                continue;
            };
            let forward = edge.from() == here;
            if (!forward && edge.oneway())
                || edge
                    .max_wingspan()
                    .is_some_and(|limit| limit < request.wingspan)
            {
                continue;
            }
            let next = if forward { edge.to() } else { edge.from() };
            if settled[next.as_usize()] {
                continue;
            }
            let duration = millis(edge.length().value() / request.speed);
            let Some(leave) = departure(
                table,
                request,
                here,
                current.time,
                (edge_id, forward, next),
                duration,
            ) else {
                continue;
            };
            let penalty = match edge.kind() {
                EdgeKind::Runway => RUNWAY_PENALTY - 1.0,
                EdgeKind::LeadIn if here != request.from && next != request.to => {
                    LEAD_IN_PENALTY - 1.0
                }
                _ => 0.0,
            } + (request.surcharge)(edge_id);
            #[allow(clippy::cast_precision_loss)]
            let cost =
                current.cost + (leave + duration - current.time) as f64 + penalty * duration as f64;
            if cost < best[next.as_usize()] {
                best[next.as_usize()] = cost;
                labels.push(Label {
                    node: next.as_usize(),
                    time: leave + duration,
                    cost,
                    previous: Some(label),
                    via: Some((edge_id, forward, leave)),
                });
                heap.push(Queued {
                    cost: cost + estimate(next.as_usize()),
                    label: labels.len() - 1,
                });
            }
        }
    }
    trace(&labels, found?, request)
}

/// The route ending at label `found`, from its predecessors.
fn trace<B, C>(labels: &[Label], found: usize, request: &PlanRequest<B, C>) -> Option<TimedRoute> {
    let mut steps = Vec::new();
    let mut cursor = Some(found);
    let mut leave_next = labels.get(found)?.time + request.stay;
    while let Some(index) = cursor {
        let label = labels[index];
        steps.push(Step {
            node: Id::new(u32::try_from(label.node).ok()?),
            arrive: label.time,
            leave: leave_next,
            via: label.via.map(|(edge, forward, _)| (edge, forward)),
        });
        if let Some((_, _, left)) = label.via {
            leave_next = left;
        }
        cursor = label.previous;
    }
    steps.reverse();
    Some(TimedRoute {
        steps,
        open: request.open,
    })
}

/// Pairs of conflicting reservations between different owners, for tests.
#[cfg(test)]
pub(crate) fn conflicts(table: &Reservations) -> usize {
    use super::HEADWAY;

    let mut found = 0;
    for windows in &table.nodes {
        for (i, a) in windows.iter().enumerate() {
            for b in &windows[i + 1..] {
                if a.owner != b.owner && a.from < b.until && b.from < a.until {
                    found += 1;
                }
            }
        }
    }
    for passes in &table.edges {
        for (i, a) in passes.iter().enumerate() {
            for b in &passes[i + 1..] {
                if a.owner == b.owner {
                    continue;
                }
                let clash = if a.forward == b.forward {
                    (a.enter - b.enter).abs() < HEADWAY - 1 || (a.exit - b.exit).abs() < HEADWAY - 1
                } else {
                    a.enter < b.exit && b.enter < a.exit
                };
                if clash {
                    found += 1;
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        ground::{AirportGroundDefinition, EdgeDefinition, definition::Point},
        test_data::data,
    };

    /// A `size` × `size` grid of two-way taxiways ~100 m apart near
    /// Frankfurt; node `i` at column `i % size`, row `i / size`.
    fn grid(size: u32) -> Result<AirportGround> {
        let mut nodes: Vec<Point> = Vec::new();
        let mut edges = Vec::new();
        for row in 0..size {
            for column in 0..size {
                nodes.push([
                    50.03 + f64::from(row) * 0.0009,
                    8.56 + f64::from(column) * 0.0014,
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
                aprons: Vec::new(),
            },
            &data()?.world,
        )
    }

    /// Nothing is ever blocked.
    type Never = fn(Id<Node>, i64, i64) -> bool;
    /// No edge costs extra.
    type Open = fn(Id<Edge>) -> f64;

    fn request(from: u32, to: u32, start: i64, owner: u32) -> PlanRequest<Never, Open> {
        PlanRequest {
            from: Id::new(from),
            to: Id::new(to),
            start,
            wingspan: Meters::from_raw(36.0),
            speed: 5.0,
            owner,
            stay: 0,
            open: false,
            deadline: start + 3_600_000,
            start_wait: i64::MAX,
            start_shared: false,
            blocked: |_, _, _| false,
            surcharge: |_| 0.0,
            goal_first_from: None,
        }
    }

    #[test]
    fn test_plan_route_goes_straight_when_alone() -> Result<()> {
        let ground = grid(3)?;
        let table = Reservations::new(ground.nodes().len(), ground.edges().len());
        let route = plan_route(&ground, &table, &request(0, 2, 0, 1)).context("a route")?;
        let nodes: Vec<u32> = route.steps.iter().map(|s| s.node.index()).collect();
        assert_eq!(nodes, [0, 1, 2]);
        // About 200 m at 5 m/s.
        assert!(
            (39_000..=41_000).contains(&route.arrival()),
            "{}",
            route.arrival()
        );
        let schedule = route.schedule(&ground);
        assert_eq!(schedule.first().map(|b| b.0), Some(0));
        assert!((along_at(&schedule, 20_000) - 100.0).abs() < 2.0);
        assert!(along_at(&schedule, -5).abs() < 1e-9);
        assert!((along_at(&schedule, 1_000_000) - 200.0).abs() < 2.0);
        assert!(along_at(&[], 5).abs() < 1e-9);
        assert_eq!(route.points_until(&ground, 1).len(), 2);
        assert!((schedule.last().map_or(0.0, |b| b.1) - 200.0).abs() < 2.0);
        assert_eq!(route.points(&ground).len(), 3);
        Ok(())
    }

    #[test]
    fn test_plan_route_can_take_the_goal_before_later_bookings() -> Result<()> {
        let ground = grid(3)?;
        let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
        // Someone booked node 2 from 30 s to 90 s, someone else from 100 s.
        table.occupy(Id::new(2), 30_000, 90_000, 5);
        table.occupy(Id::new(2), 100_000, 160_000, 6);
        let mut first = request(0, 2, 0, 1);
        first.stay = 30_000;
        let waited = plan_route(&ground, &table, &first).context("after them")?;
        assert!(waited.arrival() >= 160_000, "{}", waited.arrival());
        // Held up long enough, it goes first at its goal.
        first.goal_first_from = Some(0);
        let ahead = plan_route(&ground, &table, &first).context("first")?;
        assert!(ahead.arrival() < 50_000, "{}", ahead.arrival());
        // Not past someone already waiting there.
        table.hold(Id::new(2), 0, 20_000, 7);
        let behind = plan_route(&ground, &table, &first).context("behind")?;
        assert!(behind.arrival() >= 20_000, "{}", behind.arrival());
        Ok(())
    }

    #[test]
    fn test_plan_route_avoids_costly_edges() -> Result<()> {
        let ground = grid(3)?;
        let table = Reservations::new(ground.nodes().len(), ground.edges().len());
        // Edge 0 joins nodes 0 and 1, on the straight way.
        let mut around = request(0, 2, 0, 1);
        around.surcharge = |edge| if edge.index() == 0 { 100.0 } else { 0.0 };
        let route = plan_route(&ground, &table, &around).context("a detour")?;
        assert!(
            route
                .steps
                .iter()
                .all(|s| s.via.is_none_or(|(e, _)| e.index() != 0))
        );
        assert_eq!(route.steps.last().map(|s| s.node.index()), Some(2));
        // Where every way costs extra, the shortest is still taken.
        around.surcharge = |_| 100.0;
        let straight = plan_route(&ground, &table, &around).context("a route")?;
        assert_eq!(straight.steps.len(), 3);
        Ok(())
    }

    #[test]
    fn test_plan_route_waits_or_detours_around_others() -> Result<()> {
        let ground = grid(3)?;
        let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
        // The first crosses the middle node 4 from 3 to 5.
        let first = plan_route(&ground, &table, &request(3, 5, 0, 1)).context("first")?;
        table.reserve(&first, 1);
        // The second goes 1 → 7 through the middle at the same time.
        let second = plan_route(&ground, &table, &request(1, 7, 0, 2)).context("second")?;
        table.reserve(&second, 2);
        assert_eq!(conflicts(&table), 0);
        assert!(second.arrival() > 40_000, "it had to wait or go round");
        // Head-on along the top row: the second waits for the first.
        let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
        let east = plan_route(&ground, &table, &request(0, 2, 0, 1)).context("east")?;
        table.reserve(&east, 1);
        let west = plan_route(&ground, &table, &request(2, 0, 0, 2)).context("west")?;
        table.reserve(&west, 2);
        assert_eq!(conflicts(&table), 0);
        Ok(())
    }

    #[test]
    fn test_open_goals_and_blocked_nodes_hold_traffic() -> Result<()> {
        let ground = grid(3)?;
        let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
        let mut holder = request(0, 1, 0, 1);
        holder.stay = 120_000;
        holder.open = true;
        let first = plan_route(&ground, &table, &holder).context("holder")?;
        table.reserve(&first, 1);
        assert!(table.windows(Id::new(1)).iter().any(|w| w.open));
        // The next to node 1 arrives only after the estimate.
        let next = plan_route(&ground, &table, &request(3, 1, 0, 2)).context("next")?;
        assert!(
            next.arrival() >= first.arrival() + 120_000,
            "{}",
            next.arrival()
        );
        // A node blocked until 100 s is passed after that.
        let table = Reservations::new(ground.nodes().len(), ground.edges().len());
        let mut runway = request(0, 2, 0, 3);
        let gate = |node: Id<Node>, from: i64, _until: i64| node.index() == 1 && from < 100_000;
        let blocked = PlanRequest {
            blocked: gate,
            surcharge: runway.surcharge,
            goal_first_from: runway.goal_first_from,
            from: runway.from,
            to: runway.to,
            start: runway.start,
            wingspan: runway.wingspan,
            speed: runway.speed,
            owner: runway.owner,
            stay: runway.stay,
            open: runway.open,
            deadline: runway.deadline,
            start_wait: runway.start_wait,
            start_shared: runway.start_shared,
        };
        let crossing = plan_route(&ground, &table, &blocked).context("crossing")?;
        let at = crossing
            .steps
            .iter()
            .find(|s| s.node.index() == 1)
            .map_or(0, |s| s.arrive);
        assert!(
            at >= 100_000 || crossing.steps.iter().all(|s| s.node.index() != 1),
            "{at}"
        );
        runway.deadline = 10_000;
        assert!(plan_route(&ground, &table, &runway).is_none(), "too late");
        Ok(())
    }

    #[test]
    fn test_start_wait_and_shared_start() -> Result<()> {
        let ground = grid(3)?;
        let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
        // Someone holds node 1 for a minute: leaving 0 towards 2 waits.
        table.hold(Id::new(1), 0, 60_000, 9);
        let mut eager = request(0, 2, 0, 1);
        eager.start_wait = 10_000;
        let blocked_ahead = plan_route(&ground, &table, &eager);
        assert!(
            blocked_ahead.is_none_or(|r| r.steps.iter().all(|s| s.node.index() != 1)),
            "no long wait at the start"
        );
        // Standing together with someone at node 0: allowed to leave.
        table.hold(Id::new(0), 0, 60_000, 8);
        let mut shared = request(0, 2, 0, 1);
        assert!(plan_route(&ground, &table, &shared).is_none());
        shared.start_shared = true;
        assert!(plan_route(&ground, &table, &shared).is_some());
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_planned_routes_never_conflict(
            requests in prop::collection::vec((0_u32..16, 0_u32..16, 0_i64..120_000), 1..10),
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let ground = grid(4).map_err(fail)?;
            let mut table = Reservations::new(ground.nodes().len(), ground.edges().len());
            let mut planned = 0;
            for (owner, (from, to, start)) in (1_u32..).zip(requests.iter().copied()) {
                if from == to {
                    continue;
                }
                if let Some(route) = plan_route(&ground, &table, &request(from, to, start, owner)) {
                    prop_assert_eq!(route.steps[0].node, Id::new(from));
                    prop_assert_eq!(route.steps[route.steps.len() - 1].node, Id::new(to));
                    prop_assert!(route.steps.windows(2).all(|w| w[0].leave <= w[1].arrive));
                    table.reserve(&route, owner);
                    planned += 1;
                }
                prop_assert_eq!(conflicts(&table), 0);
            }
            prop_assert!(planned > 0 || requests.iter().all(|r| r.0 == r.1));
        }
    }
}
