//! The ground controller's clearances, every tick (plan M4 D79, D81, D82):
//! aircraft near their clearance limit ask to be cleared on, the most urgent
//! first. A clearance runs to the next point where the aircraft can stop
//! without blocking a junction, crosses a runway in one go when the runway
//! controller allows it, and is given only when nobody is in the way and
//! nobody comes the other way on the route ahead.

use super::{
    AIRCRAFT_LENGTH, DEADLOCK_AFTER, LOOKAHEAD, STUCK_AFTER, Target, Taxi, ms, taxi_millis, time_of,
};
use crate::{
    SimDuration, SimTime,
    ground::clearance::{Clearances, Hop},
    ops::{Phase, control::RunwayKey, fleet::Operations},
};

/// Nobody may come the other way on this much of the route beyond a
/// clearance, m: aircraft never meet head-on.
const AHEAD: f64 = 300.0;

/// An aircraft vacating a runway is clear of it this far beyond the last
/// point on it, m: a runway-vacated line is about this far from the
/// centreline.
const VACATED: f64 = 90.0;

/// How urgent a request is (D81), most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Urgency {
    /// On a runway: vacating or crossing.
    OnRunway,
    /// Holding short of a runway to cross it.
    Crossing,
    Departure,
    Arrival,
}

/// What clearing an aircraft on would take.
struct Batch {
    /// Hops to clear, from the next one to `last`.
    first: usize,
    last: usize,
    /// Hops after `last` it intends to take, up to `ahead`.
    ahead: usize,
    /// The runway it crosses, and until when.
    crossing: Option<(RunwayKey, SimTime)>,
}

impl Operations {
    /// Clears every taxiing aircraft on as far as the way allows, the most
    /// urgent first; ties by how long they have waited, then tail index.
    pub(in crate::ops::fleet) fn grant_clearances(&mut self, now: SimTime) {
        self.find_deadlocks(now);
        self.wanted.clear();
        self.wanted_edges.clear();
        let mut requests = std::mem::take(&mut self.requests);
        requests.clear();
        for index in 0..self.taxis.len() {
            if let Some(urgency) = self.request(index) {
                let since = self.taxis[index]
                    .as_ref()
                    .and_then(|taxi| taxi.waiting_since)
                    .unwrap_or(ms(now));
                requests.push((urgency as u8, since, index));
            }
        }
        requests.sort_unstable();
        for &(_, _, index) in &requests {
            self.clear_on(index, now);
        }
        self.requests = requests;
    }

    /// Marks the least urgent aircraft of every cycle of aircraft waiting
    /// for each other to give way (D83); after `DEADLOCK_AFTER` it is
    /// cleared regardless. Marks those queueing behind a departure at its
    /// holding position too: the runway moves that queue.
    fn find_deadlocks(&mut self, now: SimTime) {
        /// Cycles longer than this are not looked for.
        const LONGEST: usize = 16;
        for index in 0..self.taxis.len() {
            let mut cycle = false;
            let mut gives_way = true;
            let mine = (self.urgency(index), index);
            let mut current = index;
            for _ in 0..LONGEST {
                let Some(next) = self.taxis[current]
                    .as_ref()
                    .and_then(|taxi| taxi.waiting_for)
                    .map(|owner| owner as usize)
                else {
                    break;
                };
                if next == index {
                    cycle = true;
                    break;
                }
                gives_way &= (self.urgency(next), next) < mine;
                current = next;
            }
            let queued = current != index
                && self.taxis[current].as_ref().is_some_and(|taxi| {
                    taxi.target == Target::Hold && taxi.granted >= taxi.hops.len()
                });
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.deadlock_since = if cycle && gives_way {
                    Some(taxi.deadlock_since.unwrap_or(ms(now)))
                } else {
                    None
                };
                taxi.queued = !cycle && queued;
            }
        }
    }

    /// How urgent tail `index`'s clearances are.
    fn urgency(&self, index: usize) -> Urgency {
        let Some(taxi) = self.taxis[index].as_ref() else {
            return Urgency::Arrival;
        };
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return Urgency::Arrival;
        };
        let here = taxi
            .granted
            .checked_sub(1)
            .and_then(|i| taxi.hops.get(i))
            .or_else(|| taxi.hops.first())
            .and_then(|hop| hop.node);
        let next = taxi.hops.get(taxi.granted).and_then(|hop| hop.node);
        if airside.runway_at(here).is_some() {
            Urgency::OnRunway
        } else if airside.runway_at(next).is_some() {
            Urgency::Crossing
        } else if taxi.target == Target::Hold {
            Urgency::Departure
        } else {
            Urgency::Arrival
        }
    }

    /// Whether tail `index` asks to be cleared on, and how urgently.
    fn request(&self, index: usize) -> Option<Urgency> {
        let taxi = self.taxis[index].as_ref()?;
        let motion = self.tails[index].motion.as_ref()?;
        let moving = matches!(
            motion.phase(),
            Phase::Pushback | Phase::TaxiOut | Phase::TaxiIn
        );
        if !moving || taxi.waiting.is_some() || taxi.granted >= taxi.hops.len() {
            return None;
        }
        if taxi.granted > 0 && taxi.limit() - motion.along() > LOOKAHEAD {
            return None;
        }
        Some(self.urgency(index))
    }

    /// Clears tail `index` on while the way is free and its limit is near.
    fn clear_on(&mut self, index: usize, now: SimTime) {
        let along = self.tails[index]
            .motion
            .as_ref()
            .map_or(0.0, crate::ops::motion::Motion::along);
        loop {
            let Some(taxi) = self.taxis[index].as_ref() else {
                return;
            };
            let first = taxi.granted;
            if first >= taxi.hops.len() || (first > 0 && taxi.limit() - along > LOOKAHEAD) {
                return;
            }
            if !self.clear_through(index, first, along, now) {
                return;
            }
        }
    }

    /// Clears tail `index` on to at least hop `through` if the way is free
    /// (or, after waiting too long, regardless: D75); `false`, holding short
    /// of a busy runway and counting its wait, otherwise.
    pub(super) fn clear_through(
        &mut self,
        index: usize,
        through: usize,
        along: f64,
        now: SimTime,
    ) -> bool {
        match self.assess(index, through, along, now) {
            Some(batch) => {
                self.commit(index, &batch, now);
                true
            }
            None => false,
        }
    }

    /// The batch clearing tail `index` on to at least hop `through` would
    /// take, if the way is free.
    fn assess(&mut self, index: usize, through: usize, along: f64, now: SimTime) -> Option<Batch> {
        let owner = Self::owner(index);
        let taxi = self.taxis[index].as_ref()?;
        let airside = self.ground.airside(taxi.airport)?;
        let clearances = &airside.clearances;
        let hops = &taxi.hops;
        let first = taxi.granted;
        if first >= hops.len() {
            return None;
        }
        // Leaving the runway it started on (an arrival vacating) is no
        // crossing.
        let own = airside.runway_at(hops.first().and_then(|hop| hop.node));
        let here = first
            .checked_sub(1)
            .and_then(|i| hops.get(i))
            .and_then(|hop| hop.node);
        let crossing = airside
            .runway_at(hops[first].node)
            .filter(|&r| Some(r) != airside.runway_at(here) && Some(r) != own);
        let mut last = through.clamp(first, hops.len() - 1);
        if let Some(runway) = crossing {
            // Across the runway up to the first point off it.
            while last + 1 < hops.len()
                && hops[last]
                    .node
                    .is_none_or(|n| airside.runway_at(Some(n)) == Some(runway))
            {
                last += 1;
            }
        }
        let on_runway = |i: usize| airside.runway_at(hops[i].node).is_some();
        if crossing.is_none() && (on_runway(first) || first.checked_sub(1).is_some_and(on_runway)) {
            // Vacating: clear of the runway, to the first point beyond that
            // is no junction, holding short of the taxiway there if need be.
            while last + 1 < hops.len() && on_runway(last + 1) {
                last += 1;
            }
            let off = hops[last].along + VACATED;
            while last + 1 < hops.len()
                && (on_runway(last)
                    || hops[last].along < off
                    || clearances.is_junction(hops[last].point))
            {
                last += 1;
            }
        } else {
            // On to a point where it can stop without blocking a junction.
            while last + 1 < hops.len() && !clearances.stoppable(hops[last].point) {
                last += 1;
            }
        }
        let mut ahead = last;
        while ahead + 1 < hops.len() && hops[ahead + 1].along <= hops[last].along + AHEAD {
            ahead += 1;
        }
        let escape = taxi.escaped || overdue(taxi, now);
        let blocker = (first..=last)
            .find_map(|i| clearances.blocker(owner, &hops[i], None))
            .or_else(|| {
                (first..=ahead).find_map(|i| {
                    hops[i]
                        .via
                        .and_then(|(e, f)| clearances.opposed(owner, e, f))
                })
            })
            .or_else(|| self.wanted_by(index, &hops[first..=last], &hops[first..=ahead]));
        let in_way = !escape && blocker.is_some();
        let until = time_of(ms(now) + taxi_millis(hops[last].along - along + AIRCRAFT_LENGTH));
        let key = crossing.map(|runway| RunwayKey {
            airport: taxi.airport,
            runway,
        });
        let runway_busy =
            !escape && key.is_some_and(|key| !self.control.may_cross(key, now, until));
        if in_way || runway_busy {
            if let Some(key) = key.filter(|_| runway_busy) {
                self.hold_short(index, key, now);
            }
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.waiting_since.get_or_insert(ms(now));
                taxi.waiting_for = blocker.filter(|_| in_way);
            }
            if let Some(taxi) = self.taxis[index].as_ref() {
                let airport = taxi.airport.as_usize();
                self.wanted.extend(
                    taxi.hops[first..=last]
                        .iter()
                        .map(|hop| (airport, owner, hop.point)),
                );
                self.wanted_edges.extend(
                    taxi.hops[first..=ahead]
                        .iter()
                        .filter_map(|hop| hop.via.map(|(e, f)| (airport, owner, e.index(), f))),
                );
            }
            return None;
        }
        Some(Batch {
            first,
            last,
            ahead,
            crossing: key.map(|key| (key, until)),
        })
    }

    /// A more urgent aircraft refused this tick that waits for a point at
    /// or near one of `hops`, which tail `index` would take, or to take an
    /// edge of `ahead` the other way: not if that
    /// aircraft waits on it, directly or down the line, for then it has to
    /// get out of the way.
    fn wanted_by(&self, index: usize, hops: &[Hop], ahead: &[Hop]) -> Option<u32> {
        let owner = Self::owner(index);
        let airport = self.taxis[index].as_ref()?.airport;
        let clearances = &self.ground.airside(airport)?.clearances;
        let here = airport.as_usize();
        let points = self.wanted.iter().find(|&&(at, wanter, point)| {
            at == here
                && wanter != owner
                && hops.iter().any(|hop| clearances.close(hop.point, point))
                && !self.waits_on(wanter, owner)
        });
        let edges = || {
            self.wanted_edges
                .iter()
                .find(|&&(at, wanter, edge, forward)| {
                    at == here
                        && wanter != owner
                        && ahead.iter().any(|hop| {
                            hop.via
                                .is_some_and(|(e, f)| e.index() == edge && f != forward)
                        })
                        && !self.waits_on(wanter, owner)
                })
        };
        points
            .map(|&(_, wanter, _)| wanter)
            .or_else(|| edges().map(|&(_, wanter, _, _)| wanter))
    }

    /// Whether tail `wanter` waits on tail `owner`, directly or through
    /// others waiting in turn.
    fn waits_on(&self, wanter: u32, owner: u32) -> bool {
        /// Chains longer than this are not followed.
        const LONGEST: usize = 16;
        let mut current = wanter;
        for _ in 0..LONGEST {
            let Some(next) = self
                .taxis
                .get(current as usize)
                .and_then(Option::as_ref)
                .and_then(|taxi| taxi.waiting_for)
            else {
                return false;
            };
            if next == owner {
                return true;
            }
            current = next;
        }
        false
    }

    /// Clears tail `index` through `batch`.
    fn commit(&mut self, index: usize, batch: &Batch, now: SimTime) {
        let owner = Self::owner(index);
        if let Some((key, until)) = batch.crossing {
            self.control.crossing(key, until);
        }
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let escape = !taxi.escaped && overdue(taxi, now);
        let airport = taxi.airport;
        if let Some(since) = taxi.waiting_since {
            self.stats
                .waited_for_clearance(airport, SimDuration::from_millis(ms(now) - since));
        }
        if let Some(airside) = self.ground.airside_mut(airport) {
            taxi.take(&mut airside.clearances, owner, batch);
        }
        if escape {
            taxi.escaped = true;
            self.stats.ground_conflict(airport);
        }
    }

    /// Tail `index` holds short of runway `key` and asks to cross; take-offs
    /// give way once it has waited long enough (plan M4 D77).
    fn hold_short(&mut self, index: usize, key: RunwayKey, now: SimTime) {
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let since = match taxi.crossing {
            Some((held, since)) if held == key => since,
            _ => ms(now),
        };
        taxi.crossing = Some((key, since));
        self.control.crossing_wanted(key, time_of(since), now);
    }
}

/// Whether `taxi` is to be cleared regardless: it gives way in a deadlock
/// that has lasted, or has waited far too long (D75) other than in a
/// runway queue.
fn overdue(taxi: &Taxi, now: SimTime) -> bool {
    taxi.deadlock_since
        .is_some_and(|since| ms(now) - since >= DEADLOCK_AFTER)
        || (!taxi.queued
            && taxi
                .waiting_since
                .is_some_and(|since| ms(now) - since >= STUCK_AFTER))
}

impl Taxi {
    /// Takes the hops of `batch` and intends the edges after them.
    fn take(&mut self, clearances: &mut Clearances, owner: u32, batch: &Batch) {
        for hop in &self.hops[batch.first..=batch.last] {
            clearances.enter(owner, hop);
            if let Some((edge, _)) = hop.via {
                clearances.forget(owner, edge);
            }
        }
        for hop in &self.hops[batch.last + 1..=batch.ahead] {
            if let Some((edge, forward)) = hop.via {
                clearances.intend(owner, edge, forward);
            }
        }
        self.granted = batch.last + 1;
        self.waiting_since = None;
        self.waiting_for = None;
        self.deadlock_since = None;
        self.crossing = None;
    }
}
