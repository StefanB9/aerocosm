//! Ground control at mapped airports (plan M4 D69, D75–D84, D89, D93), as
//! in real operations: at each junction the approaches with a holding
//! position give way and are cleared in when the junction is free and no
//! through traffic is close; the through taxiway has right of way. Pilots
//! keep their distance behind the aircraft ahead, and each aircraft's route
//! ahead locks the direction of its taxiways against head-on meetings.

use std::collections::BTreeMap;

mod arrival;
mod control;
mod departure;
mod drive;
mod reroute;
mod state;
mod tight;

use super::Operations;
use crate::{
    Id, LatLon, SimTime,
    geo::Polyline,
    ground::{
        AirportGround, Edge, EdgeKind, GroundName, GroundRoute, Node, Stand, TAXI_SPEED,
        junctions::{Junctions, Passage, Run},
    },
    ops::{Phase, TaxiGoal, active_runways_now, control::RunwayKey},
    scenario::Scenario,
    world::Airport,
};

/// An aircraft's length behind its reference point (an A321 is 45 m long),
/// m.
const AIRCRAFT_LENGTH: f64 = 45.0;
/// Pilots keep this far behind the aircraft ahead, m.
const FOLLOW: f64 = 60.0;
/// A junction's zone reaches this far beyond its nodes along a path: an
/// aircraft whose body is within it is in the junction, m.
const ZONE: f64 = 30.0;
/// Before a junction someone else is in, an aircraft holds this far short
/// of its nodes, wingtip-clear of whoever passes through, m.
const HOLD_SHORT: f64 = 45.0;
/// An aircraft this close to a junction zone's edge, m, has not entered
/// the zone (it stopped there).
const EDGE: f64 = 0.5;
/// Through traffic this close to a junction keeps a yielding aircraft
/// waiting at its hold, m.
const GAP: f64 = 150.0;
/// An aircraft locks the direction of its taxiways this far ahead where no
/// hold comes first, m.
const LOCK_AHEAD: f64 = 600.0;
/// A pushback needs the taxilane clear this far either side of where it
/// ends, m.
const PUSH_CLEAR: f64 = 120.0;
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

/// Who stands where on one chain, this tick: tail, position along the
/// chain, and whether it moves the chain's way.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Occupant {
    tail: u32,
    position: f64,
}

/// One mapped airport's ground control state.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Airside {
    junctions: Junctions,
    /// Per edge, its name and whether it is part of a runway, and its
    /// length, m.
    names: Vec<(Option<GroundName>, bool)>,
    lengths: Vec<f64>,
    /// For each node, the index of the active runway it lies on.
    runway_of: Vec<Option<usize>>,
    /// Per junction, the aircraft cleared into it from a hold.
    claims: Vec<Option<u32>>,
    /// Per chain, who has locked it and which way.
    locks: Vec<Vec<(u32, bool)>>,
    /// Per chain, the direction recent traffic took it, from -1 (against
    /// its way) to 1 (D84).
    flow: Vec<f32>,
    /// This tick: who is taxiing here, who is in each junction, and who is
    /// on each chain where.
    taxiing: Vec<u32>,
    /// Where each of them is, as pilots see it.
    seen: Vec<drive::Seen>,
    occupied: Vec<Vec<u32>>,
    on_chain: Vec<Vec<Occupant>>,
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

    /// Someone other than `owner` who has locked `chain` the other way than
    /// `forward`.
    fn opposed(&self, owner: u32, chain: u32, forward: bool) -> Option<u32> {
        self.locks
            .get(chain as usize)?
            .iter()
            .find(|&&(other, way)| other != owner && way != forward)
            .map(|&(other, _)| other)
    }

    /// Locks `chain` for `owner` going `forward`, learning its flow.
    fn lock(&mut self, owner: u32, chain: u32, forward: bool) {
        if let Some(locks) = self.locks.get_mut(chain as usize)
            && !locks.iter().any(|&(other, _)| other == owner)
        {
            locks.push((owner, forward));
            if let Some(flow) = self.flow.get_mut(chain as usize) {
                let way = if forward { 1.0 } else { -1.0 };
                *flow = 0.9 * *flow + 0.1 * way;
            }
        }
    }

    /// Frees `owner`'s lock on `chain`.
    fn unlock(&mut self, owner: u32, chain: u32) {
        if let Some(locks) = self.locks.get_mut(chain as usize) {
            locks.retain(|&(other, _)| other != owner);
        }
    }

    /// How much traffic has recently taken `chain` against `forward`, 0–1.
    fn against_flow(&self, chain: u32, forward: bool) -> f64 {
        let flow = f64::from(self.flow.get(chain as usize).copied().unwrap_or(0.0));
        let way = if forward { flow } else { -flow };
        (-way).max(0.0)
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
            let junctions = Junctions::new(layout);
            if let Some(slot) = airports.get_mut(airport.as_usize()) {
                *slot = Some(Airside {
                    names: layout
                        .edges()
                        .iter()
                        .map(|e| (e.name(), e.kind() == EdgeKind::Runway))
                        .collect(),
                    lengths: layout.edges().iter().map(|e| e.length().value()).collect(),
                    claims: vec![None; junctions.junctions().len()],
                    taxiing: Vec::new(),
                    seen: Vec::new(),
                    occupied: vec![Vec::new(); junctions.junctions().len()],
                    locks: vec![Vec::new(); junctions.chains().len()],
                    flow: vec![0.0; junctions.chains().len()],
                    on_chain: vec![Vec::new(); junctions.chains().len()],
                    junctions,
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
            for claim in &mut airside.claims {
                if *claim == Some(owner) {
                    *claim = None;
                }
            }
            for locks in &mut airside.locks {
                locks.retain(|&(other, _)| other != owner);
            }
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
    /// Where it ends, as the flight panel names it (D85).
    goal: TaxiGoal,
    /// The stand a departure pushes back from, and how far the push goes,
    /// m.
    stand: Option<GroundName>,
    push: f64,
    /// The route's edges, the first taxied from `offset` metres along the
    /// path, and its last node.
    edges: Vec<(Id<Edge>, bool)>,
    offset: f64,
    end_node: Option<Id<Node>>,
    /// The junctions it passes and the chains it runs along.
    passages: Vec<Passage>,
    runs: Vec<Run>,
    /// The next passage it has to be cleared into (all before it are),
    /// and how many runs it has locked.
    next: usize,
    locked: usize,
    /// Where it may taxi up to now, m, and the aircraft it follows there.
    limit: f64,
    leader: Option<u32>,
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
    /// How often it has been sent another way out of a deadlock (D93).
    reroutes: u8,
}

impl Taxi {
    /// Where it may taxi up to, m.
    fn limit(&self) -> f64 {
        self.limit
    }

    /// The next passage it has to be cleared into, if any.
    fn next_hold(&self) -> Option<(usize, f64)> {
        self.passages
            .iter()
            .enumerate()
            .skip(self.next)
            .find_map(|(i, p)| p.hold.map(|h| (i, h)))
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
        self.route_avoiding(layout, index, (from, to), |_, _| false)
    }

    /// Like `route_for`, never along the edges `avoid(edge, forward)`.
    fn route_avoiding(
        &self,
        layout: &AirportGround,
        index: usize,
        (from, to): (Id<Node>, Id<Node>),
        avoid: impl Fn(Id<Edge>, bool) -> bool,
    ) -> Option<GroundRoute> {
        let owner = Self::owner(index);
        let airside = self.ground.airside(layout.airport());
        layout.route_with(from, to, self.wingspans[index], |edge, forward| {
            if avoid(edge, forward) {
                return f64::INFINITY;
            }
            airside.map_or(0.0, |a| {
                a.junctions.chain(edge).map_or(0.0, |(chain, way)| {
                    let forward = way == forward;
                    AGAINST_FLOW * a.against_flow(chain, forward)
                        + if a.opposed(owner, chain, forward).is_some() {
                            AGAINST_TRAFFIC
                        } else {
                            0.0
                        }
                })
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
        let (passages, runs) = airside
            .junctions
            .passages(layout, from, route.edges(), offset);
        let route_end = offset + route.length().value();
        let end_node = route.edges().last().and_then(|&(e, forward)| {
            layout
                .edge(e)
                .map(|e| if forward { e.to() } else { e.from() })
        });
        let mut points = base;
        points.extend(route.points().iter().skip(usize::from(!points.is_empty())));
        let more: f64 = extension
            .windows(2)
            .map(|w| w[0].distance_to(w[1]).value())
            .sum();
        points.extend(extension.iter().skip(1));
        let end = route_end + more;
        let goal = match target {
            Target::Hold => TaxiGoal::HoldingPoint(
                end_node
                    .and_then(|node| layout.holding_positions().iter().find(|h| h.node() == node))
                    .map(crate::ground::HoldingPosition::runway),
            ),
            Target::Stand(stand) => layout
                .stands()
                .get(stand.as_usize())
                .map_or(TaxiGoal::Spot, |s| TaxiGoal::Stand(s.name())),
            Target::Spot(_) => TaxiGoal::Spot,
        };
        self.taxis[index] = Some(Taxi {
            airport,
            target,
            goal,
            stand: None,
            push: 0.0,
            edges: route.edges().to_vec(),
            offset,
            end_node,
            passages,
            runs,
            next: 0,
            locked: 0,
            limit: offset,
            leader: None,
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
            reroutes: 0,
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
