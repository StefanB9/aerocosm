//! Ground control at mapped airports (plan M4 D69, D75–D84): a ground
//! controller clears each aircraft along its route hop by hop, as far as the
//! way is free or used in the same direction; aircraft taxi up to their
//! clearance limit and hold there.

use std::collections::BTreeMap;

mod arrival;
mod departure;
mod drive;
mod grant;

use super::Operations;
use crate::{
    Id, LatLon, SimTime,
    geo::Polyline,
    ground::{
        AirportGround, GroundRoute, Node, Stand, TAXI_SPEED,
        clearance::{Clearances, Hop},
    },
    ops::{Phase, active_runways_now, control::RunwayKey},
    scenario::Scenario,
    world::Airport,
};

/// Aircraft closer than this would overlap (an A321 is 45 m long): nodes
/// this close are held together, and an aircraft holds what lies this far
/// behind it, m.
const AIRCRAFT_LENGTH: f64 = 45.0;
/// An aircraft asks to be cleared on once it is this close to its limit,
/// so that it rarely has to stop, m.
const LOOKAHEAD: f64 = 120.0;
/// A cycle of aircraft waiting for each other this long is a deadlock: its
/// least urgent member is cleared regardless (D75, D83), ms.
const DEADLOCK_AFTER: i64 = 60_000;
/// Waiting this long for a clearance for any reason, an aircraft taxies
/// regardless as a last resort (D75), ms.
const STUCK_AFTER: i64 = 1_800_000;
/// A departure waiting at its gate for a pushback clearance this long goes
/// regardless (D75), ms.
const GATE_ESCAPE_AFTER: i64 = 1_800_000;
/// A departure without a pushback clearance asks again after this long, ms.
const GATE_RETRY: i64 = 30_000;
/// An arrival at a waiting spot looks for a stand this often, ms.
const RETRY: i64 = 15_000;
/// An arrival waiting at a spot this long shares a stand (D69), ms.
const SPOT_ESCAPE_AFTER: i64 = 1_800_000;
/// A departure leaves its stand only if it would wait at most this long at
/// the holding position; queues build at the gates, ms.
const MAX_TAXI_WAIT: i64 = 120_000;
/// Shortest and longest expected take-off interval from one holding
/// position, ms; between them it is learnt from line-ups.
const DEPARTURE_INTERVAL: i64 = 60_000;
const MAX_INTERVAL: i64 = 300_000;
/// Routes cost this many times an edge's length more against the direction
/// traffic has recently taken it…
const AGAINST_FLOW: f64 = 3.0;
/// …and this much more where someone is on it, or cleared onto it, the other
/// way.
const AGAINST_TRAFFIC: f64 = 10.0;
/// Pushback speed, m/s, and engine start, s (as in motion).
const PUSH_SPEED: f64 = 1.0;
const ENGINE_START: f64 = 60.0;

fn ms(time: SimTime) -> i64 {
    time.as_millis()
}

fn time_of(millis: i64) -> SimTime {
    SimTime::from_millis(millis).unwrap_or(SimTime::UNIX_EPOCH)
}

/// How long taxiing `metres` takes unimpeded, ms.
fn taxi_millis(metres: f64) -> i64 {
    // SAFETY: airport distances, a few kilometres at taxi speed.
    #[allow(clippy::cast_possible_truncation)]
    let millis = (metres.max(0.0) / TAXI_SPEED * 1000.0) as i64;
    millis
}

/// One mapped airport's ground control state.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Airside {
    clearances: Clearances,
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

    /// The active runway `node` lies on, if any.
    fn runway_at(&self, node: Option<Id<Node>>) -> Option<usize> {
        node.and_then(|n| self.runway_of.get(n.as_usize()).copied().flatten())
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
                    clearances: Clearances::new(
                        layout,
                        &layout.close_nodes(AIRCRAFT_LENGTH),
                        AIRCRAFT_LENGTH,
                    ),
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

    /// Frees everything tail `owner` holds at `airport`.
    pub(super) fn release(&mut self, airport: Id<Airport>, owner: u32) {
        if let Some(airside) = self.airside_mut(airport) {
            airside.clearances.release(owner);
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
    target: Target,
    /// The route's clearance points, along the path's polyline.
    hops: Vec<Hop>,
    /// Hops cleared so far, and hops passed (and freed) so far.
    granted: usize,
    passed: usize,
    /// The polyline taxied, and where it ends (past the last hop where it
    /// lines up beyond the holding position), m.
    points: Vec<LatLon>,
    end: f64,
    /// When the taxi started and how long it would take unimpeded, ms.
    started: i64,
    unimpeded: i64,
    /// Since when it has waited for its next clearance, ms, and for whom.
    waiting_since: Option<i64>,
    waiting_for: Option<u32>,
    /// Since when it has been the one to give way in a deadlock, ms.
    deadlock_since: Option<i64>,
    /// Waiting in a queue for a runway, behind a departure at its holding
    /// position.
    queued: bool,
    /// Waiting at a spot for a stand, trying again at this time, ms.
    waiting: Option<i64>,
    /// When it reached the spot, ms.
    spot_since: i64,
    /// Cleared regardless after waiting too long (D75): it no longer waits
    /// for anyone.
    escaped: bool,
    /// Holding short of a runway to cross it, since this time, ms.
    crossing: Option<(RunwayKey, i64)>,
}

impl Taxi {
    /// Where it may taxi up to: its last cleared point, or the end, m.
    fn limit(&self) -> f64 {
        if self.granted >= self.hops.len() {
            self.end
        } else {
            self.granted
                .checked_sub(1)
                .and_then(|i| self.hops.get(i))
                .map_or(f64::NEG_INFINITY, |hop| hop.along)
        }
    }
}

/// How a taxi starts: the route, its goal, the polyline already taxied and
/// its length, and any stretch beyond the last node.
struct Start {
    route: GroundRoute,
    from: Id<Node>,
    target: Target,
    base: Vec<LatLon>,
    offset: f64,
    extension: Vec<LatLon>,
}

impl Operations {
    fn owner(index: usize) -> u32 {
        u32::try_from(index).unwrap_or(u32::MAX)
    }

    /// The route from `from` to `to` for tail `index`: round traffic coming
    /// the other way, and with the flow the airport's traffic has settled
    /// into, as published taxi flows would have it (D84).
    fn route_for(
        &self,
        layout: &AirportGround,
        index: usize,
        from: Id<Node>,
        to: Id<Node>,
    ) -> Option<GroundRoute> {
        let owner = Self::owner(index);
        let clearances = self.ground.airside(layout.airport()).map(|a| &a.clearances);
        layout.route_with(from, to, self.wingspans[index], |edge, forward| {
            clearances.map_or(0.0, |c| {
                AGAINST_FLOW * c.against_flow(edge, forward)
                    + if c.opposed(owner, edge, forward).is_some() {
                        AGAINST_TRAFFIC
                    } else {
                        0.0
                    }
            })
        })
    }

    /// Starts tail `index` on `start` at `airport`, nothing cleared yet;
    /// returns its polyline.
    fn start_taxi(
        &mut self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        start: Start,
        now: SimTime,
    ) -> Option<Polyline> {
        let Start {
            route,
            from,
            target,
            base,
            offset,
            extension,
        } = start;
        let layout = scenario.ground().airport(airport)?;
        let airside = self.ground.airside(airport)?;
        let hops = airside.clearances.hops(layout, from, route.edges(), offset);
        let mut points = base;
        points.extend(route.points().iter().skip(usize::from(!points.is_empty())));
        let more: f64 = extension
            .windows(2)
            .map(|w| w[0].distance_to(w[1]).value())
            .sum();
        points.extend(extension.iter().skip(1));
        let end = hops.last().map_or(offset, |hop| hop.along) + more;
        self.taxis[index] = Some(Taxi {
            airport,
            target,
            hops,
            granted: 0,
            passed: 0,
            end,
            started: ms(now),
            unimpeded: taxi_millis(end - offset),
            waiting_since: None,
            waiting_for: None,
            deadlock_since: None,
            queued: false,
            waiting: None,
            spot_since: 0,
            escaped: false,
            crossing: None,
            points: points.clone(),
        });
        Polyline::new(&points).ok()
    }

    /// Whether tail `index` taxies cleared regardless of others (D75).
    pub(super) fn escaped(&self, index: usize) -> bool {
        self.taxis[index].as_ref().is_some_and(|taxi| taxi.escaped)
    }

    /// Whether tail `index` is on a controlled taxi in `phase`.
    pub(super) fn controlled(&self, index: usize, phase: Phase) -> bool {
        matches!(phase, Phase::TaxiOut | Phase::TaxiIn) && self.taxis[index].is_some()
    }
}
