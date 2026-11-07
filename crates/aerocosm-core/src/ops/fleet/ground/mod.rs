//! Ground control at mapped airports (plan M4 PR 3, D69, D70, D75–D78):
//! every taxi is planned with time reservations, followed to the second and
//! re-planned when something ahead takes longer than planned.

use std::collections::BTreeMap;

mod arrival;
mod departure;
mod drive;

use super::Operations;
use crate::{
    Id, LatLon, SimTime,
    geo::Polyline,
    ground::{
        Node, Stand, TAXI_SPEED,
        reserve::{NODE_BUFFER, PlanRequest, Reservations, TimedRoute, plan_route},
    },
    ops::{
        Phase, active_runways_now,
        control::{Control, RunwayKey},
    },
    scenario::Scenario,
    world::Airport,
};

/// Plans may run this far ahead, ms.
const HORIZON: i64 = 30 * 60_000;
/// A departure is expected to wait this long at the holding position, ms.
/// Short, so the holding position passes a departure a minute; real waits
/// stretch the window while the aircraft is still there.
const QUEUE_ESTIMATE: i64 = 30_000;
/// While still waiting, an open window always reaches this far ahead, ms.
const OPEN_MARGIN: i64 = 20_000;
/// An arrival is expected to wait this long at a waiting spot, ms.
const SPOT_ESTIMATE: i64 = 300_000;
/// An arrival waits at most this long at its runway exit, ms.
const EXIT_WAIT: i64 = 45_000;
/// A departure without a taxi plan asks again after this long, ms.
const GATE_RETRY: i64 = 30_000;
/// Blocked aircraft try again this often, ms.
const RETRY: i64 = 15_000;
/// After this long without a plan an aircraft taxies regardless (D75), ms.
const ESCAPE_AFTER: i64 = 600_000;
/// A departure leaves its stand only if its plan waits at most this long on
/// the way to the holding position; queues build at the gates, ms.
const MAX_TAXI_WAIT: i64 = 120_000;
/// Shortest and longest expected take-off interval from one holding
/// position, ms; between them it is learnt from line-ups.
const DEPARTURE_INTERVAL: i64 = 60_000;
const MAX_INTERVAL: i64 = 300_000;
/// A departure held at its gate this long goes regardless (D75), ms.
const GATE_ESCAPE_AFTER: i64 = 1_800_000;
/// Pushback speed, m/s, and engine start, s (as in motion).
const PUSH_SPEED: f64 = 1.0;
const ENGINE_START: f64 = 60.0;

/// Time a route spends waiting before its last node, ms.
fn waits(route: &TimedRoute) -> i64 {
    let steps = &route.steps;
    steps
        .iter()
        .take(steps.len().saturating_sub(1))
        .map(|step| step.leave - step.arrive)
        .sum()
}

fn ms(time: SimTime) -> i64 {
    time.as_millis()
}

fn time_of(millis: i64) -> SimTime {
    SimTime::from_millis(millis).unwrap_or(SimTime::UNIX_EPOCH)
}

/// One mapped airport's ground control state.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Airside {
    table: Reservations,
    /// For each node, the index of the active runway it lies on.
    runway_of: Vec<Option<usize>>,
    spots: Vec<Id<Node>>,
    /// The tail waiting at each spot.
    spot_owner: Vec<Option<u32>>,
    /// Per holding position: the earliest expected take-off of the next
    /// departure there, ms (departure management).
    next_departure: BTreeMap<Id<Node>, i64>,
    /// Per holding position: the last line-up and the observed interval
    /// between line-ups while a queue waited, ms.
    pace: BTreeMap<Id<Node>, (i64, i64)>,
}

impl Airside {
    /// The expected interval between departures from `hold`, ms.
    fn interval(&self, hold: Id<Node>) -> i64 {
        self.pace.get(&hold).map_or(DEPARTURE_INTERVAL, |p| p.1)
    }
}

/// Ground control at every mapped airport, by airport index.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GroundControl {
    airports: Vec<Option<Airside>>,
}

impl GroundControl {
    /// Ground control for `scenario`'s mapped airports with the runways in
    /// use now.
    pub(super) fn new(scenario: &Scenario) -> Self {
        let world = scenario.world();
        let mut airports = vec![None; world.airport_count()];
        for layout in scenario.ground().airports() {
            let airport = layout.airport();
            let Some(place) = world.airport(airport) else {
                continue;
            };
            let runways = world.runways_of(airport);
            let modes = scenario
                .traffic()
                .map_or(&[][..], |t| t.targets().runway_modes(place.ident()));
            let mut runway_of = vec![None; layout.nodes().len()];
            let mut active: Vec<usize> = active_runways_now(runways, modes)
                .iter()
                .map(|a| a.runway)
                .collect();
            active.dedup();
            for runway in active {
                for node in layout.nodes_on(&runways[runway]) {
                    runway_of[node.as_usize()] = Some(runway);
                }
            }
            let on_runway: Vec<bool> = runway_of.iter().map(Option::is_some).collect();
            let spots = layout.waiting_spots(&on_runway);
            if let Some(slot) = airports.get_mut(airport.as_usize()) {
                *slot = Some(Airside {
                    table: Reservations::new(layout.nodes().len(), layout.edges().len()),
                    runway_of,
                    spot_owner: vec![None; spots.len()],
                    spots,
                    next_departure: BTreeMap::new(),
                    pace: BTreeMap::new(),
                });
            }
        }
        Self { airports }
    }

    fn airside(&self, airport: Id<Airport>) -> Option<&Airside> {
        self.airports.get(airport.as_usize())?.as_ref()
    }

    fn airside_mut(&mut self, airport: Id<Airport>) -> Option<&mut Airside> {
        self.airports.get_mut(airport.as_usize())?.as_mut()
    }

    /// Forgets old reservations and keeps waiting aircraft's windows running.
    pub(super) fn refresh(&mut self, now: SimTime) {
        for airside in self.airports.iter_mut().flatten() {
            airside.table.refresh_open(ms(now), OPEN_MARGIN);
            airside.table.prune(ms(now) - NODE_BUFFER);
        }
    }

    /// Frees everything tail `owner` holds at `airport`.
    pub(super) fn release(&mut self, airport: Id<Airport>, owner: u32) {
        if let Some(airside) = self.airside_mut(airport) {
            airside.table.release(owner);
            for slot in &mut airside.spot_owner {
                if *slot == Some(owner) {
                    *slot = None;
                }
            }
        }
    }

    /// Whether `airport` is under ground control.
    pub(super) fn controls(&self, airport: Id<Airport>) -> bool {
        self.airside(airport).is_some()
    }
}

/// Where a taxi ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The runway holding position.
    Hold,
    Stand(Id<Stand>),
    /// A waiting spot, until a stand frees.
    Spot(Id<Node>),
}

/// A tail's controlled taxi.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Taxi {
    airport: Id<Airport>,
    route: TimedRoute,
    schedule: Vec<(i64, f64)>,
    /// Distance along the path's polyline where the route starts, and the
    /// polyline before it.
    offset: f64,
    base: Vec<LatLon>,
    target: Target,
    /// Steps whose node has been left.
    departed: usize,
    /// While blocked: since when, and when to try again.
    blocked: Option<(i64, i64)>,
    /// When the taxi started and how long it would take unimpeded, ms.
    started: i64,
    unimpeded: i64,
    /// Waiting at a spot for a stand, trying again at this time.
    waiting: Option<i64>,
}

/// How a taxi starts: its goal, the polyline already taxied and its length,
/// and any stretch beyond the last node.
struct Start {
    target: Target,
    base: Vec<LatLon>,
    offset: f64,
    extension: Vec<LatLon>,
}

/// The parts of a request that do not depend on the planning attempt.
struct Plan {
    /// Longest wait at the start, ms, and whether the start is shared.
    start_wait: i64,
    shared: bool,
    airport: Id<Airport>,
    from: Id<Node>,
    to: Id<Node>,
    start: i64,
    stay: i64,
    open: bool,
}

impl Operations {
    fn owner(index: usize) -> u32 {
        u32::try_from(index).unwrap_or(u32::MAX)
    }

    /// Plans `plan` for tail `index` around everyone's reservations, or
    /// around nobody's when `escape` (D75).
    fn plan_taxi(
        &self,
        scenario: &Scenario,
        index: usize,
        plan: &Plan,
        escape: bool,
    ) -> Option<TimedRoute> {
        let layout = scenario.ground().airport(plan.airport)?;
        let airside = self.ground.airside(plan.airport)?;
        let empty;
        let table = if escape {
            empty = Reservations::new(layout.nodes().len(), layout.edges().len());
            &empty
        } else {
            &airside.table
        };
        let control: &Control = &self.control;
        let runway_of = &airside.runway_of;
        let airport = plan.airport;
        let blocked = |node: Id<Node>, from: i64, until: i64| {
            !escape
                && runway_of
                    .get(node.as_usize())
                    .copied()
                    .flatten()
                    .is_some_and(|runway| {
                        !control.may_cross(
                            RunwayKey { airport, runway },
                            time_of(from),
                            time_of(until),
                        )
                    })
        };
        plan_route(
            layout,
            table,
            &PlanRequest {
                from: plan.from,
                to: plan.to,
                start: plan.start,
                wingspan: self.wingspans[index],
                speed: TAXI_SPEED,
                owner: Self::owner(index),
                stay: plan.stay,
                open: plan.open,
                deadline: plan.start + HORIZON,
                start_wait: plan.start_wait,
                start_shared: plan.shared,
                blocked,
            },
        )
    }

    /// Reserves `route` and starts following it; returns its polyline.
    fn start_taxi(
        &mut self,
        scenario: &Scenario,
        index: usize,
        plan: &Plan,
        route: TimedRoute,
        start: Start,
    ) -> Option<Polyline> {
        let Start {
            target,
            base,
            offset,
            extension,
        } = start;
        let layout = scenario.ground().airport(plan.airport)?;
        let owner = Self::owner(index);
        let airside = self.ground.airside_mut(plan.airport)?;
        airside.table.reserve(&route, owner);
        let mut points = base.clone();
        let tail_points = route.points(layout);
        points.extend(tail_points.iter().skip(usize::from(!points.is_empty())));
        let mut schedule = route.schedule(layout);
        // On past the last node, unreserved: the holding position's open
        // window keeps followers behind it.
        let more: f64 = extension
            .windows(2)
            .map(|w| w[0].distance_to(w[1]).value())
            .sum();
        if more > 0.0
            && let Some(&(time, along)) = schedule.last()
        {
            points.extend(extension.iter().skip(1));
            // SAFETY: metres at taxi speed.
            #[allow(clippy::cast_possible_truncation)]
            let extra = (more / TAXI_SPEED * 1000.0).ceil() as i64;
            schedule.push((time + extra, along + more));
        }
        let length = schedule.last().map_or(0.0, |s| s.1);
        // SAFETY: taxi durations are minutes.
        #[allow(clippy::cast_possible_truncation)]
        let unimpeded = (length / TAXI_SPEED * 1000.0) as i64;
        self.taxis[index] = Some(Taxi {
            airport: plan.airport,
            schedule,
            offset,
            base,
            target,
            departed: 0,
            blocked: None,
            started: plan.start,
            unimpeded,
            waiting: None,
            route,
        });
        Polyline::new(&points).ok()
    }

    /// Whether tail `index` is on a controlled taxi in `phase`.
    pub(super) fn controlled(&self, index: usize, phase: Phase) -> bool {
        matches!(phase, Phase::TaxiOut | Phase::TaxiIn) && self.taxis[index].is_some()
    }
}
