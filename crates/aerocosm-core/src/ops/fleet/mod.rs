//! The operations controller (D48): every tail flies its weekly rotation,
//! leg after leg, with delays carried along the rotation.

use anyhow::{Context, Result};

pub(super) mod board;
mod flow;
mod ground;
mod recovery;
mod remote;
mod runways;
mod spares;
mod stands;
mod start;

use recovery::Ready;
use start::Rotation;

use self::{
    flow::Flow,
    ground::{GroundControl, Table, Taxi},
    remote::{Parking, next_look},
    stands::StandBook,
};
use super::{
    Phase, StartMode, active_runways_now,
    control::{Control, RunwayKey},
    motion::{Event, Motion, Profile},
    path::{Leg, LegGround, Origin, Path, plan_leg},
    stats::{DelayCauses, Statistics},
    tail::{draw_delay, draw_taxi_in, draw_taxi_out, next_leg, off_block},
    timing::{NoTimer, Part, Timer},
    view::{Airframe, FlightView, Holding, Recovery},
};
use crate::{
    Id, RngSeed, SimDuration, SimRng, SimTime, StreamId, TICK, WakeCategory,
    airline::AirlineIcao,
    ground::{Stand, TAXI_SPEED},
    scenario::Scenario,
    schedule::{Schedule, ScheduledFlight, Tail, min_turnaround},
    time::{LocalDate, Zone},
    units::{Meters, MetersPerSecond},
    world::{Airport, AirportKind, RunwayDesignator, RunwayEnd},
};

/// Old landing slots are forgotten every this many milliseconds.
const PRUNE_EVERY: i64 = 600_000;
/// Boarding starts this long before off-block.
const BOARDING: SimDuration = SimDuration::from_minutes(30);
/// Where one tail is in its rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TailOps {
    /// Index of the current leg in the tail's rotation.
    leg: usize,
    /// Local Monday of the week the current leg belongs to.
    week: LocalDate,
    scheduled: SimTime,
    off_block: SimTime,
    boarding_from: SimTime,
    /// How long after schedule the aircraft was ready for this leg (may be
    /// negative), and the airline's own delay drawn for it: what the
    /// departure delay is split by (plan M8 D167).
    late: SimDuration,
    drawn: SimDuration,
    /// `None` while parked before boarding.
    motion: Option<Motion>,
}

/// Every tail flying its rotation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Operations {
    time: SimTime,
    schedule: Schedule,
    tails: Vec<TailOps>,
    /// The geometry of each tail's current leg, while it boards or flies.
    paths: Vec<Option<Path>>,
    /// The profile of each tail's current leg, valid while it boards or
    /// flies; kept apart from the geometry because every tick reads it.
    profiles: Vec<Profile>,
    zones: Vec<Zone>,
    turnarounds: Vec<SimDuration>,
    callsigns: Vec<AirlineIcao>,
    major: Vec<bool>,
    rng: SimRng,
    movements: Vec<u32>,
    airborne: usize,
    flights_flown: u64,
    /// Runway control at the sequenced airports.
    control: Control,
    /// Runway control state of each tail's current leg.
    flows: Vec<Flow>,
    /// Wake category of each tail's aircraft.
    wakes: Vec<WakeCategory>,
    /// Wingspan of each tail's aircraft, for stands and taxi routes.
    wingspans: Vec<Meters>,
    /// Whether each tail's airline flies cargo (its stands, plan M5 D120).
    cargo: Vec<bool>,
    /// Stands taken at mapped airports (plan M4).
    stands: StandBook,
    /// The stand each tail stands on, while it is on one.
    parked: Vec<Option<(Id<Airport>, Id<Stand>)>>,
    /// Where each tail on the ground is parked: its stand or remote (plan M5
    /// D125).
    parking: Vec<Parking>,
    /// Ground control at mapped airports (plan M4 PR 3).
    ground: GroundControl,
    /// Each tail's controlled taxi, while it taxies under control.
    taxis: Table<Taxi>,
    /// Since when each tail waits at its stand for a taxi plan, ms.
    gate_blocked: Vec<Option<i64>>,
    /// When each tail last chose its runways again at the gate, ms (plan
    /// M6 D151).
    rechosen: Vec<i64>,
    /// Clearance requests of the tick, reused (urgency, since, tail).
    requests: Vec<(u8, i64, usize)>,
    /// When each tail is next looked at: a parked one when something is due,
    /// any other every tick.
    look_at: Vec<SimTime>,
    /// The tails that taxi under control, this tick; reused.
    taxiing: Vec<usize>,
    /// Points refused aircraft wait for, this tick: (airport, tail, point).
    /// Nobody less urgent is cleared onto them (D81).
    wanted: Vec<(usize, u32, u32)>,
    /// Edges refused aircraft wait to take this tick, and which way:
    /// (airport, tail, edge, forward). Nobody less urgent is cleared onto
    /// them the other way.
    wanted_edges: Vec<(usize, u32, u32, bool)>,
    /// The rotation (schedule tail) each aircraft flies now, `None` for a
    /// spare (plan M8 D170), and the aircraft flying each rotation (D169).
    /// The schedule's tails come first, then its spares.
    rotation_of: Vec<Option<usize>>,
    aircraft_of: Vec<usize>,
    /// Each aircraft's base.
    bases: Vec<Id<Airport>>,
    /// Aircraft relieved by a spare on their way in: they hand their
    /// rotation over at the gate (plan M8 D170).
    relieved: Vec<bool>,
    /// Cancelled legs, (rotation, week, leg), with their scheduled
    /// departure: boards show them (plan M8 D173). Pruned after a day.
    cancelled: std::collections::BTreeMap<(usize, LocalDate, usize), SimTime>,
    /// How each aircraft came to its current round trip by recovery, and
    /// the trip's return leg (week, leg): noted until then (plan M8 D173).
    took_over: Vec<Option<(Recovery, LocalDate, usize)>>,
    /// The aircraft of each pool (airline, base and type), which may swap
    /// rotations, and each aircraft's pool.
    pools: Vec<Vec<usize>>,
    pool_of: Vec<usize>,
    /// Punctuality, delays and movements since the start.
    stats: Statistics,
    /// Take-offs and touchdowns at controlled runways, for tests.
    #[cfg(test)]
    log: Vec<RunwayUse>,
}

/// A take-off or touchdown on a controlled runway, for tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct RunwayUse {
    key: super::control::RunwayKey,
    time: SimTime,
    wake: WakeCategory,
    /// The approach speed of a touchdown; `None` for a take-off.
    approach_speed: Option<f64>,
}

fn add(time: SimTime, duration: SimDuration) -> Result<SimTime> {
    time.checked_add(duration)
        .with_context(|| format!("{time} + {duration} is out of range"))
}

/// Taxi-out time along a routed taxi-out, pushback excluded, s.
fn routed_taxi_out(path: &Path, profile: &Profile) -> Option<f64> {
    path.routed_out()
        .then(|| (profile.taxi_out_length() - profile.pushback_length()).max(0.0) / TAXI_SPEED)
}

fn seconds_between(from: SimTime, to: SimTime) -> f64 {
    to.duration_since(from).as_secs_f64()
}

impl Operations {
    /// Operations for `schedule` from `start`, placed according to `mode`.
    pub(crate) fn new(
        scenario: &Scenario,
        schedule: Schedule,
        start: SimTime,
        seed: RngSeed,
        mode: StartMode,
    ) -> Result<Self> {
        let traffic = scenario
            .traffic()
            .context("operations need traffic inputs")?;
        let world = scenario.world();
        let rotations = schedule.tails().len();
        let tail_count = rotations + schedule.spares().len();
        let Facts {
            zones,
            turnarounds,
            wakes,
            wingspans,
            cargoes,
            bases,
        } = facts(scenario, &schedule)?;
        let (pools, pool_of) = pools(&schedule);
        let sequenced: Vec<Id<Airport>> = traffic
            .targets()
            .airports()
            .filter_map(|target| world.airport_by_ident(target.airport()))
            .collect();
        let control = runway_control(scenario, &sequenced);
        let mut operations = Self {
            time: start,
            tails: Vec::with_capacity(tail_count),
            paths: vec![None; tail_count],
            profiles: vec![Profile::default(); tail_count],
            zones,
            turnarounds,
            callsigns: traffic
                .airlines()
                .iter()
                .map(crate::airline::Airline::icao)
                .collect(),
            major: world
                .airports()
                .map(|(_, airport)| airport.kind() == AirportKind::Major)
                .collect(),
            rng: seed.stream(StreamId::Operations),
            movements: vec![0; world.airport_count()],
            airborne: 0,
            flights_flown: 0,
            control,
            flows: vec![Flow::default(); tail_count],
            wakes,
            wingspans,
            cargo: cargoes,
            stands: StandBook::new(scenario),
            parked: vec![None; tail_count],
            ground: GroundControl::new(scenario),
            taxis: Table::new(tail_count),
            gate_blocked: vec![None; tail_count],
            rechosen: vec![i64::MIN; tail_count],
            parking: vec![Parking::Stand; tail_count],
            requests: Vec::with_capacity(tail_count),
            taxiing: Vec::new(),
            look_at: vec![start; tail_count],
            wanted: Vec::new(),
            wanted_edges: Vec::new(),
            pools,
            pool_of,
            rotation_of: (0..rotations)
                .map(Some)
                .chain(std::iter::repeat_n(None, tail_count - rotations))
                .collect(),
            aircraft_of: (0..rotations).collect(),
            bases,
            relieved: vec![false; tail_count],
            cancelled: std::collections::BTreeMap::new(),
            took_over: vec![None; tail_count],
            stats: {
                let mut stats = Statistics::new(world.airport_count());
                stats.set_start(start);
                stats
            },
            #[cfg(test)]
            log: Vec::new(),
            schedule,
        };
        for index in 0..rotations {
            let tail = operations.place_tail(scenario, index, mode)?;
            operations.tails.push(tail);
        }
        for index in rotations..tail_count {
            let week = super::timeline::week_of(&operations.zones[index], start)?;
            let tail = operations.place_spare(scenario, index, week);
            operations.tails.push(tail);
        }
        operations.airborne = operations
            .tails
            .iter()
            .filter(|tail| tail.motion.is_some_and(|m| m.phase().is_airborne()))
            .count();
        Ok(operations)
    }

    /// The rotation aircraft `index` flies now.
    fn rotation_id(&self, index: usize) -> Result<Id<Tail>> {
        Id::from_usize(
            self.rotation_of
                .get(index)
                .context("aircraft out of range")?
                .context("a spare flies no rotation")?,
        )
    }

    /// How many aircraft fly the schedule.
    pub(crate) const fn airframe_count(&self) -> usize {
        self.tails.len()
    }

    fn rotation(&self, index: usize) -> Result<Rotation<'_>> {
        Ok(Rotation {
            flights: self.schedule.flights_of(self.rotation_id(index)?),
            zone: self.zones.get(index).context("tail out of range")?,
            turnaround: self.turnarounds[index],
        })
    }

    fn flight(&self, index: usize, leg: usize) -> Result<&ScheduledFlight> {
        self.schedule
            .flights_of(self.rotation_id(index)?)
            .get(leg)
            .context("leg out of range")
    }

    fn plan(&self, scenario: &Scenario, index: usize, leg: usize) -> Result<Leg> {
        self.plan_at(scenario, index, leg, None)
    }

    /// Plans leg `leg` of tail `index`. At the gate at `at_gate`, about to
    /// push back, the departure runways' queues count (plan M6 D151).
    fn plan_at(
        &self,
        scenario: &Scenario,
        index: usize,
        leg: usize,
        at_gate: Option<SimTime>,
    ) -> Result<Leg> {
        let flight = self.flight(index, leg)?;
        let traffic = scenario.traffic().context("traffic")?;
        let aircraft = traffic.aircraft().get(flight.aircraft()).context("type")?;
        let origin = self.parked[index]
            .filter(|(airport, _)| *airport == flight.origin())
            .map(|(_, stand)| Origin::Stand(stand))
            .or(match self.parking[index] {
                Parking::Spot(node) => Some(Origin::Node(node)),
                _ => None,
            });
        let current = self.paths[index].as_ref().filter(|_| at_gate.is_some());
        let leaving = current.and_then(Path::departure_end);
        let from = flight.origin();
        let turn_in = |end: &RunwayEnd, _: &[RunwayDesignator]| {
            at_gate.map_or(0.0, |now| {
                let wait = self.turn_in(from, end.designator(), now);
                runways::stay(wait, leaving == Some(end.designator()))
            })
        };
        plan_leg(
            scenario.world(),
            traffic.targets(),
            LegGround {
                network: scenario.ground(),
                origin,
                turn_in: at_gate.is_some().then_some(&turn_in),
            },
            aircraft,
            flight.origin(),
            flight.destination(),
        )
        .with_context(|| format!("cannot plan leg {leg} of tail {index}"))
    }

    /// The operator of leg `leg` of tail `index`.
    fn airline_of(&self, index: usize, leg: usize) -> Result<AirlineIcao> {
        let flight = self.flight(index, leg)?;
        self.callsigns
            .get(flight.airline().as_usize())
            .copied()
            .context("airline out of range")
    }

    /// Puts tail `index` on a stand at the origin of leg `leg`, if mapped.
    fn stand_for_leg(&mut self, scenario: &Scenario, index: usize, leg: usize) -> Result<()> {
        let origin = self.flight(index, leg)?.origin();
        let near = scenario
            .world()
            .airport(origin)
            .context("origin out of range")?
            .location();
        let airline = self.airline_of(index, leg)?;
        self.take_stand(scenario, index, origin, airline, (near, None));
        Ok(())
    }

    fn is_major(&self, airport: Id<Airport>) -> bool {
        self.major.get(airport.as_usize()).copied().unwrap_or(false)
    }

    /// Advances every tail by one tick.
    pub(crate) fn step(&mut self, scenario: &Scenario) -> Result<()> {
        self.step_with(scenario, &mut NoTimer)
    }

    /// Advances every tail by one tick, marking each part of the tick on
    /// `timer` (plan M5 D136).
    pub(crate) fn step_with(&mut self, scenario: &Scenario, timer: &mut impl Timer) -> Result<()> {
        let now = self.time;
        timer.enter(Part::FlowControl);
        if now.as_millis() % PRUNE_EVERY == 0 {
            self.prune_control(now)?;
        }
        timer.enter(Part::GroundControl);
        self.control_ground(now);
        for index in 0..self.tails.len() {
            // Most of the fleet is parked with nothing due (plan M5 D136).
            if self.look_at[index] > now {
                continue;
            }
            let part = match self.tails[index].motion {
                None => Part::Parked,
                Some(motion) if motion.phase() == Phase::Boarding => Part::Parked,
                Some(motion)
                    if motion.phase() == Phase::Pushback && self.taxis[index].is_some()
                        || self.controlled(index, motion.phase()) =>
                {
                    Part::Taxiing
                }
                Some(_) => Part::Flying,
            };
            timer
                .tail(index, part, || self.step_tail(scenario, index, now))
                .with_context(|| format!("tail {index} at {now}"))?;
            let tail = &self.tails[index];
            self.look_at[index] = if tail.motion.is_none() {
                next_look(self.parking[index], tail.boarding_from, tail.off_block)
            } else {
                now
            };
        }
        self.time = add(now, TICK)?;
        Ok(())
    }

    /// A tail on the ground before boarding: towed if due (D125), then on to
    /// boarding when its time comes.
    fn step_parked(&mut self, scenario: &Scenario, index: usize, now: SimTime) -> Result<()> {
        if self.is_spare(index) {
            self.step_spare(index, now);
            return Ok(());
        }
        if self.parking[index] != Parking::Stand {
            let leg = self.tails[index].leg;
            let origin = self.flight(index, leg)?.origin();
            let airline = self.airline_of(index, leg)?;
            if self.tend_parking(scenario, index, origin, airline, now) {
                return Ok(());
            }
        }
        if now >= self.tails[index].boarding_from {
            if !matches!(self.parking[index], Parking::Spot(_)) {
                self.stand_for_leg(scenario, index, self.tails[index].leg)?;
            }
            let planned = self.plan(scenario, index, self.tails[index].leg)?;
            self.tails[index].motion = Some(Motion::boarding(&planned.profile));
            self.install(index, planned);
        }
        Ok(())
    }

    fn step_tail(&mut self, scenario: &Scenario, index: usize, now: SimTime) -> Result<()> {
        // Only the motion changes on most ticks; the rest of the tail's state
        // is read in place and rewritten only when a leg changes.
        match self.tails[index].motion {
            None => self.step_parked(scenario, index, now)?,
            Some(motion) if motion.phase() == Phase::Boarding => {
                if now >= self.tails[index].off_block {
                    self.rechoose_departure(scenario, index, now)?;
                    let flight = *self.flight(index, self.tails[index].leg)?;
                    let (origin, destination) = (
                        self.is_major(flight.origin()),
                        self.is_major(flight.destination()),
                    );
                    let mut out = draw_taxi_out(&mut self.rng, origin)?;
                    let back = draw_taxi_in(&mut self.rng, destination)?;
                    if let Some(path) = self.paths[index].as_ref() {
                        out = routed_taxi_out(path, &self.profiles[index]).unwrap_or(out);
                    }
                    let start = Motion::start(&self.profiles[index], out, back);
                    let leg = self.tails[index].leg;
                    let wait = match self.regulate(index, leg, start, now)? {
                        // Wait at the gate for the calculated take-off time…
                        Some(delay) => Some(delay),
                        // …or, with no way out yet or a long runway queue,
                        // wait at the stand and ask again.
                        None => self.plan_departure(scenario, index, flight.origin(), now)?,
                    };
                    if let Some(wait) = wait {
                        let leaves = add(now, wait)?;
                        if self.cancels_at_gate(index, leaves)? {
                            return self.cancel_at_gate(index, now);
                        }
                        self.tails[index].off_block = leaves;
                        return Ok(());
                    }
                    let start = Motion::start(&self.profiles[index], out, back);
                    self.release_stand(index);
                    self.parking[index] = Parking::Stand;
                    let tail = &mut self.tails[index];
                    tail.off_block = now;
                    tail.motion = Some(start);
                    let tail = &self.tails[index];
                    let causes = DelayCauses::split(
                        now.duration_since(tail.scheduled),
                        tail.late,
                        tail.drawn,
                    );
                    self.stats.departed(flight.origin(), causes, now);
                }
            }
            Some(mut motion) => {
                let leg = self.tails[index].leg;
                let before = motion.phase();
                let event = if self.controlled(index, before) {
                    let airline = self.airline_of(index, leg)?;
                    self.drive_taxi(scenario, index, &mut motion, now, airline)
                } else {
                    self.fly(index, leg, &mut motion, now, TICK.as_secs_f64())?
                };
                if before == Phase::Queue && motion.phase() != Phase::Queue {
                    self.left_queue(index, leg, now)?;
                }
                if before == Phase::Landing && motion.phase() == Phase::Landing {
                    self.still_rolling(index, now);
                }
                self.tails[index].motion = Some(motion);
                // Most ticks have no event; only events need the flight.
                match event {
                    Some(Event::Takeoff) => {
                        self.airborne += 1;
                        let origin = self.flight(index, leg)?.origin();
                        self.count(origin);
                        self.stats.moved(origin, now);
                        self.dispatch_spare(index, leg, now)?;
                    }
                    Some(Event::Landing) => {
                        self.airborne = self.airborne.saturating_sub(1);
                        self.touched_down(index, leg, now)?;
                        let destination = self.flight(index, leg)?.destination();
                        let airline = self.airline_of(index, leg)?;
                        if !self.plan_arrival(scenario, index, destination, airline, now)
                            && let Some(seconds) =
                                self.stand_on_arrival(scenario, index, destination, airline)
                            && let Some(motion) = self.tails[index].motion.as_mut()
                        {
                            motion.set_taxi_in(seconds);
                        }
                        self.count(destination);
                        self.stats.moved(destination, now);
                    }
                    Some(Event::GoAround) => self.went_around(index, leg, now)?,
                    Some(Event::Arrived) => {
                        self.flights_flown += 1;
                        let flight = *self.flight(index, leg)?;
                        let due = add(self.tails[index].scheduled, flight.block())?;
                        self.stats
                            .arrived(flight.destination(), now.duration_since(due));
                        self.paths[index] = None;
                        let tail = self.tails[index];
                        self.tails[index] = self.next_tail_leg(index, &tail, add(now, TICK)?)?;
                        self.plan_parking(index, now);
                    }
                    None => {}
                }
            }
        }
        Ok(())
    }

    /// Tail `index` lines up on leg `leg`: its holding position is free
    /// again, and the runway's pace learns from it.
    fn left_queue(&mut self, index: usize, leg: usize, now: SimTime) -> Result<()> {
        let origin = self.flight(index, leg)?.origin();
        self.ground.release(origin, u32::try_from(index)?);
        let waited = self.taxis[index]
            .as_ref()
            .and_then(Taxi::queued_since)
            .map_or(0, |since| now.as_millis() - since);
        self.taxis.clear(index);
        let hold = self.paths[index]
            .as_ref()
            .and_then(Path::departure_nodes)
            .map(|(hold, _)| hold);
        self.lined_up(origin, hold, waited, now);
        Ok(())
    }

    /// Makes `leg` the tail's current leg.
    fn install(&mut self, index: usize, leg: Leg) {
        self.reset_flow(index);
        self.profiles[index] = leg.profile;
        self.paths[index] = Some(leg.path);
    }

    fn count(&mut self, airport: Id<Airport>) {
        if let Some(count) = self.movements.get_mut(airport.as_usize()) {
            *count += 1;
        }
    }

    /// The tail's next leg after arriving at `arrived`: the next one it
    /// flies, after any its airline cancels (plan M8 D171).
    fn next_tail_leg(&mut self, index: usize, tail: &TailOps, arrived: SimTime) -> Result<TailOps> {
        self.next_ops(index, (tail.leg, tail.week), Ready::Arrived(arrived))
    }

    /// Aircraft `index`'s state after leg `(leg, week)`, ready as `ready`:
    /// parked before the next leg it flies, or idle if a spare took its
    /// trips over (plan M8 D170), ready for others after its turnaround.
    fn next_ops(
        &mut self,
        index: usize,
        (leg, week): (usize, LocalDate),
        ready: Ready,
    ) -> Result<TailOps> {
        let since = ready.time();
        if self.hand_over(index)? {
            return Ok(TailOps::idle(week, add(since, self.turnarounds[index])?));
        }
        match self.next_flown_leg(index, leg, week, ready)? {
            Some((leg, week, scheduled, at)) => self.leg_ops((leg, week), scheduled, at, since),
            None => Ok(TailOps::idle(week, add(since, self.turnarounds[index])?)),
        }
    }

    /// A parked tail's state before leg `leg` of `week`, scheduled at
    /// `scheduled`, for which it is ready at `ready`; at the gate since
    /// `since`. Draws the leg's own delay.
    fn leg_ops(
        &mut self,
        (leg, week): (usize, LocalDate),
        scheduled: SimTime,
        ready: SimTime,
        since: SimTime,
    ) -> Result<TailOps> {
        let delay = draw_delay(&mut self.rng)?;
        let leaves = off_block(scheduled, delay, ready)?;
        Ok(TailOps {
            leg,
            week,
            scheduled,
            off_block: leaves,
            boarding_from: leaves.checked_sub(BOARDING)?.max(since),
            late: ready.duration_since(scheduled),
            drawn: delay,
            motion: None,
        })
    }

    /// The time the next step simulates.
    #[cfg(test)]
    pub(crate) const fn time(&self) -> SimTime {
        self.time
    }

    /// The schedule being flown.
    pub(crate) const fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Aircraft currently off the ground.
    pub(crate) const fn airborne(&self) -> usize {
        self.airborne
    }

    /// Take-offs plus landings at `airport` since the start.
    pub(crate) fn movements(&self, airport: Id<Airport>) -> u32 {
        self.movements.get(airport.as_usize()).copied().unwrap_or(0)
    }

    /// Punctuality, delays and movements since the start.
    pub(crate) const fn statistics(&self) -> &Statistics {
        &self.stats
    }

    /// Legs completed (on blocks) since the start.
    pub(crate) const fn flights_flown(&self) -> u64 {
        self.flights_flown
    }

    /// How aircraft `index` came to fly its current round trip, while it
    /// flies it (plan M8 D173).
    fn recovery_note(&self, index: usize) -> Option<Recovery> {
        let tail = self.tails.get(index)?;
        let (recovery, week, leg) = (*self.took_over.get(index)?)?;
        ((tail.week, tail.leg) <= (week, leg)).then_some(recovery)
    }

    /// Notes that aircraft `index` came to the round trip leaving at
    /// `(leg, week)` of its rotation by `recovery`.
    fn note_recovery(
        &mut self,
        index: usize,
        recovery: Recovery,
        (leg, week): (usize, LocalDate),
    ) -> Result<()> {
        let count = self.rotation(index)?.flights.len();
        let (back, back_week) = next_leg(leg, count, week)?;
        self.took_over[index] = Some((recovery, back_week, back));
        Ok(())
    }

    /// Every boarding or moving flight.
    pub(crate) fn flights(&self) -> impl Iterator<Item = FlightView> + '_ {
        self.tails.iter().enumerate().filter_map(|(index, tail)| {
            let motion = tail.motion?;
            let path = self.paths.get(index)?.as_ref()?;
            let flight = self.flight(index, tail.leg).ok()?;
            let (position, heading) = path.locate(&motion);
            Some(FlightView {
                airframe: Id::<Airframe>::from_usize(index).ok()?,
                tail: self.rotation_id(index).ok()?,
                callsign: self.callsign(index)?,
                airline: flight.airline(),
                aircraft: flight.aircraft(),
                kind: flight.kind(),
                origin: flight.origin(),
                destination: flight.destination(),
                phase: motion.phase(),
                position,
                heading,
                altitude: Meters::from_raw(motion.altitude()).to_feet(),
                ground_speed: MetersPerSecond::from_raw(motion.speed()).to_knots(),
                scheduled_departure: tail.scheduled,
                scheduled_arrival: tail.scheduled.checked_add(flight.block()).ok()?,
                off_block: tail.off_block,
                holding: path.hold_fix(&motion).map(|(fix, inbound)| Holding {
                    fix,
                    inbound,
                    until: self.flows[index].hold_until(),
                }),
                slot_time: self.flows[index]
                    .slot_time
                    .filter(|_| motion.phase() == Phase::Boarding),
                expected_landing: self.flows[index].landing,
                speed_factor: motion.speed_factor(),
                ground_conflict: self.escaped(index),
                taxi: self.taxi_state(index, motion.phase(), motion.speed()),
                recovery: self.recovery_note(index),
            })
        })
    }
}

/// What each aircraft, the schedule's tails then its spares, keeps from its
/// pool: zone, minimum turnaround, wake, wingspan, cargo and base.
struct Facts {
    zones: Vec<Zone>,
    turnarounds: Vec<SimDuration>,
    wakes: Vec<WakeCategory>,
    wingspans: Vec<Meters>,
    cargoes: Vec<bool>,
    bases: Vec<Id<Airport>>,
}

fn facts(scenario: &Scenario, schedule: &Schedule) -> Result<Facts> {
    let traffic = scenario
        .traffic()
        .context("operations need traffic inputs")?;
    let world = scenario.world();
    let utc = Zone::new("UTC")?;
    let count = schedule.tails().len() + schedule.spares().len();
    let mut facts = Facts {
        zones: Vec::with_capacity(count),
        turnarounds: Vec::with_capacity(count),
        wakes: Vec::with_capacity(count),
        wingspans: Vec::with_capacity(count),
        cargoes: Vec::with_capacity(count),
        bases: Vec::with_capacity(count),
    };
    for tail in schedule
        .tails()
        .map(|(_, tail)| tail)
        .chain(schedule.spares())
    {
        let base = world
            .airport(tail.base())
            .context("tail base out of range")?;
        facts
            .zones
            .push(traffic.targets().zone(base.ident()).unwrap_or(&utc).clone());
        let aircraft = traffic
            .aircraft()
            .get(tail.aircraft())
            .context("tail type")?;
        let cargo = traffic
            .airlines()
            .get(tail.airline())
            .is_some_and(|a| a.kind() == crate::airline::AirlineKind::Cargo);
        facts.turnarounds.push(min_turnaround(aircraft, cargo));
        facts.wakes.push(aircraft.wake());
        facts.wingspans.push(aircraft.wingspan());
        facts.cargoes.push(cargo);
        facts.bases.push(tail.base());
    }
    Ok(facts)
}

/// The aircraft, the schedule's tails then its spares, grouped into pools of
/// one airline, base and type (plan M8 D169), and the pool of each.
fn pools(schedule: &Schedule) -> (Vec<Vec<usize>>, Vec<usize>) {
    let count = schedule.tails().len() + schedule.spares().len();
    let mut index = std::collections::HashMap::with_capacity(count);
    let mut pools: Vec<Vec<usize>> = Vec::new();
    let mut pool_of = Vec::with_capacity(count);
    let airframes = schedule
        .tails()
        .map(|(_, tail)| tail)
        .chain(schedule.spares());
    for (tail, info) in airframes.enumerate() {
        let key = (info.airline(), info.base(), info.aircraft());
        let pool = *index.entry(key).or_insert_with(|| {
            pools.push(Vec::new());
            pools.len() - 1
        });
        pools[pool].push(tail);
        pool_of.push(pool);
    }
    (pools, pool_of)
}

/// Runway control at the `sequenced` airports, their runways used for both
/// landings and take-offs leaving room for a departure after each landing
/// (plan M7 D159).
fn runway_control(scenario: &Scenario, sequenced: &[Id<Airport>]) -> Control {
    let world = scenario.world();
    let mut control = Control::new(sequenced.iter().copied());
    let Some(traffic) = scenario.traffic() else {
        return control;
    };
    for &airport in sequenced {
        let Some(place) = world.airport(airport) else {
            continue;
        };
        let modes = traffic.targets().runway_modes(place.ident());
        for active in active_runways_now(place, world.runways_of(airport), modes) {
            if active.arrivals && active.departures {
                control.set_mixed(RunwayKey {
                    airport,
                    runway: active.runway,
                });
            }
        }
    }
    control
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed,
        schedule::{ScheduleConfig, generate},
        test_data::data,
    };

    /// Wednesday 8 July 2026, 10:00 UTC: midday traffic.
    fn midday() -> Result<SimTime> {
        "2026-07-08T10:00:00Z".parse()
    }

    pub(super) fn setup(mode: StartMode, seed: u64) -> Result<(Scenario, Operations)> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), midday()?))?;
        let operations = Operations::new(&scenario, schedule, midday()?, RngSeed::new(seed), mode)?;
        Ok((scenario, operations))
    }

    fn run(operations: &mut Operations, scenario: &Scenario, ticks: u32) -> Result<()> {
        for _ in 0..ticks {
            operations.step(scenario)?;
        }
        Ok(())
    }

    fn airborne_views(operations: &Operations) -> usize {
        operations
            .flights()
            .filter(|f| f.phase().is_airborne())
            .count()
    }

    #[test]
    fn test_warm_start_fills_the_sky() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 1)?;
        assert!(operations.airborne() > 300, "{}", operations.airborne());
        assert_eq!(airborne_views(&operations), operations.airborne());
        assert_eq!(operations.time(), midday()?);
        run(&mut operations, &scenario, 600)?;
        assert_eq!(airborne_views(&operations), operations.airborne());
        assert_eq!(
            operations.time(),
            midday()?.checked_add(SimDuration::from_minutes(10))?
        );
        Ok(())
    }

    #[test]
    fn test_cold_start_begins_on_the_ground() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Cold, 1)?;
        assert_eq!(operations.airborne(), 0);
        run(&mut operations, &scenario, 3_600)?;
        assert!(operations.airborne() > 100, "{}", operations.airborne());
        assert_eq!(airborne_views(&operations), operations.airborne());
        Ok(())
    }

    #[test]
    fn test_operations_are_deterministic() -> Result<()> {
        let (scenario, mut first) = setup(StartMode::Warm, 3)?;
        let (_, mut second) = setup(StartMode::Warm, 3)?;
        let (_, mut other) = setup(StartMode::Warm, 4)?;
        run(&mut first, &scenario, 900)?;
        run(&mut second, &scenario, 900)?;
        run(&mut other, &scenario, 900)?;
        assert_eq!(first, second);
        assert_ne!(first, other);
        Ok(())
    }

    #[test]
    fn test_movements_and_completed_flights_are_counted() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 2)?;
        run(&mut operations, &scenario, 1_800)?;
        let frankfurt = scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        // FRA handles about 1,200 movements a day: dozens per half hour.
        assert!(
            operations.movements(frankfurt) > 20,
            "{}",
            operations.movements(frankfurt)
        );
        assert!(operations.flights_flown() > 100);
        Ok(())
    }

    #[test]
    fn test_flight_views_are_plausible() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 5)?;
        run(&mut operations, &scenario, 120)?;
        let traffic = scenario.traffic().context("traffic")?;
        let mut seen = 0;
        for flight in operations.flights() {
            seen += 1;
            let origin = scenario
                .world()
                .airport(flight.origin())
                .context("origin")?;
            let airline = traffic
                .airlines()
                .get(flight.airline())
                .context("airline")?;
            assert!(
                flight
                    .callsign()
                    .to_string()
                    .starts_with(&airline.icao().to_string())
            );
            let latitude = flight.position().latitude().value();
            assert!((-90.0..=90.0).contains(&latitude), "{latitude}");
            assert!(flight.scheduled_arrival() > flight.scheduled_departure());
            assert!(flight.off_block() >= flight.scheduled_departure());
            match flight.phase() {
                Phase::Boarding => {
                    assert!(flight.position().distance_to(origin.location()).value() < 1.0);
                    assert!(flight.ground_speed().value().abs() < 1e-9);
                }
                // Short hops in light aircraft cruise low.
                Phase::Cruise => assert!(flight.altitude().value() > 2_000.0),
                // Twin Otters approach at 80 kt.
                phase if phase.is_airborne() => assert!(flight.ground_speed().value() > 60.0),
                _ => {}
            }
        }
        assert!(seen > operations.airborne());
        Ok(())
    }

    /// Checks every controlled runway's log from `from` on: landings keep wake
    /// spacing, take-offs keep departure spacing.
    fn assert_separated(operations: &Operations, from: SimTime) -> Result<usize> {
        use std::collections::HashMap;

        use crate::ops::control::{RunwayKey, arrival_spacing};
        let mut by_runway: HashMap<RunwayKey, Vec<&RunwayUse>> = HashMap::new();
        for entry in operations.log.iter().filter(|entry| entry.time >= from) {
            by_runway.entry(entry.key).or_default().push(entry);
        }
        let tolerance = SimDuration::from_secs(1);
        let mut checked = 0;
        for uses in by_runway.values() {
            let landings: Vec<_> = uses.iter().filter(|u| u.approach_speed.is_some()).collect();
            for pair in landings.windows(2) {
                let speed = pair[1].approach_speed.unwrap_or_default();
                let needed = arrival_spacing(pair[0].wake, pair[1].wake, speed);
                let gap = pair[1].time.duration_since(pair[0].time);
                anyhow::ensure!(
                    gap.as_millis() + tolerance.as_millis() >= needed.as_millis(),
                    "landings {gap} apart, {needed} needed at {}",
                    pair[1].time
                );
                checked += 1;
            }
            let takeoffs: Vec<_> = uses.iter().filter(|u| u.approach_speed.is_none()).collect();
            for pair in takeoffs.windows(2) {
                let needed = crate::ops::control::departure_spacing(pair[0].wake, pair[1].wake);
                let gap = pair[1].time.duration_since(pair[0].time);
                anyhow::ensure!(gap >= needed, "take-offs {gap} apart, {needed} needed");
                checked += 1;
            }
        }
        Ok(checked)
    }

    #[test]
    fn test_controlled_runways_keep_separation() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 7)?;
        run(&mut operations, &scenario, 7_200)?;
        // Aircraft already on final at the warm start were never sequenced.
        let settled = midday()?.checked_add(SimDuration::from_minutes(10))?;
        let checked = assert_separated(&operations, settled)?;
        assert!(checked > 200, "{checked} pairs checked");
        Ok(())
    }

    /// Frankfurt squeezed onto one runway for arrivals and departures.
    fn single_runway_frankfurt() -> Result<(Scenario, Operations)> {
        let data = data()?;
        let mut definition = data.traffic.to_definition();
        for target in &mut definition.airports {
            if target.airport.to_string() == "EDDF" {
                for mode in &mut target.runway_modes {
                    mode.arrivals.truncate(1);
                    mode.departures = mode.arrivals.clone();
                }
            }
        }
        let traffic = crate::traffic::TrafficTargets::from_definition(definition)?;
        let scenario = Scenario::with_traffic(data.world, data.aircraft, data.airlines, traffic)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), midday()?))?;
        let operations = Operations::new(
            &scenario,
            schedule,
            midday()?,
            RngSeed::new(2),
            StartMode::Warm,
        )?;
        Ok((scenario, operations))
    }

    #[test]
    fn test_congestion_is_absorbed_by_holding() -> Result<()> {
        let (scenario, mut operations) = single_runway_frankfurt()?;
        let mut held = 0;
        for _ in 0..12 {
            run(&mut operations, &scenario, 600)?;
            held = held.max(
                operations
                    .flights()
                    .filter(|f| f.phase() == Phase::Holding)
                    .count(),
            );
        }
        assert!(held > 0, "a single runway at FRA must make arrivals hold");
        let settled = midday()?.checked_add(SimDuration::from_minutes(10))?;
        assert_separated(&operations, settled)?;
        assert!(
            operations.flights_flown() > 500,
            "{}",
            operations.flights_flown()
        );
        Ok(())
    }

    #[test]
    fn test_statistics_follow_the_traffic() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 11)?;
        run(&mut operations, &scenario, 3_600)?;
        let network = operations.statistics().network();
        assert!(network.departures() > 300, "{}", network.departures());
        assert!(network.arrivals() > 300, "{}", network.arrivals());
        let on_time = network.on_time_departures();
        assert!((0.5..=1.0).contains(&on_time), "{on_time}");
        let frankfurt = scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let at = operations.statistics().airport(frankfurt);
        assert!(at.busiest_hour() > 30, "{}", at.busiest_hour());
        assert!(at.departures() > 10 && at.arrivals() > 10);
        Ok(())
    }

    #[test]
    fn test_departure_delay_is_split_by_cause() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 11)?;
        run(&mut operations, &scenario, 3_600)?;
        let network = operations.statistics().network();
        let causes = network.departure_delay_causes();
        // Averages of a split that adds up per departure, to the rounding.
        let gap = causes.total().as_millis() - network.average_departure_delay().as_millis();
        assert!(
            gap.abs() <= 3,
            "{causes:?} against {}",
            network.average_departure_delay()
        );
        assert!(causes.airline() > SimDuration::ZERO, "{causes:?}");
        assert!(causes.late_aircraft() > SimDuration::ZERO, "{causes:?}");
        Ok(())
    }

    #[test]
    fn test_congested_destinations_hold_departures_on_the_ground() -> Result<()> {
        let (scenario, mut operations) = single_runway_frankfurt()?;
        // Six hours: with holding aircraft taking slots freed ahead (plan M7
        // D159) the queue builds more slowly.
        run(&mut operations, &scenario, 6 * 3_600)?;
        let frankfurt = scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let at = operations.statistics().airport(frankfurt);
        assert!(
            at.ground_delays() > 0,
            "flights to FRA must get slots: {} arrivals, {} departures",
            at.arrivals(),
            at.departures()
        );
        assert!(at.ground_delay_minutes() > 10.0 * f64::from(at.ground_delays()) - 1e-6);
        let settled = midday()?.checked_add(SimDuration::from_minutes(10))?;
        assert_separated(&operations, settled)?;
        Ok(())
    }

    #[test]
    fn test_flight_views_show_holding_slots_and_sequence() -> Result<()> {
        let (scenario, mut operations) = single_runway_frankfurt()?;
        let frankfurt = scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let origin = scenario
            .world()
            .airport(frankfurt)
            .context("FRA")?
            .location();
        let (mut holding, mut slots, mut sequenced) = (0, 0, 0);
        for _ in 0..12 {
            run(&mut operations, &scenario, 600)?;
            let now = operations.time();
            for view in operations.flights() {
                if let Some(hold) = view.holding() {
                    holding += 1;
                    assert_eq!(view.phase(), Phase::Holding);
                    if view.destination() == frankfurt {
                        assert!(hold.fix().distance_to(origin).value() < 150_000.0);
                    }
                }
                if let Some(ctot) = view.slot_time() {
                    assert_eq!(view.phase(), Phase::Boarding);
                    assert!(ctot >= now);
                    slots += 1;
                }
                if view.expected_landing().is_some() {
                    sequenced += 1;
                }
                assert!((0.8..=1.0).contains(&view.speed_factor()));
            }
        }
        assert!(
            holding > 0 && slots > 0 && sequenced > 0,
            "{holding} {slots} {sequenced}"
        );
        Ok(())
    }
}
