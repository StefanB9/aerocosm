//! The ground controller, every tick (plan M4 D79, D81, D83, D93): where
//! everyone is, what they have left behind, and who may enter the junction
//! ahead of their hold, the most urgent first.

use super::{
    AIRCRAFT_LENGTH, DEADLOCK_AFTER, FOLLOW, GAP, LOCK_AHEAD, LOOKAHEAD, Occupant, PUSH_CLEAR,
    STUCK_AFTER, Target, Taxi, ZONE, ms, taxi_millis, time_of,
};
use crate::{
    SimDuration, SimTime,
    ops::{Phase, control::RunwayKey, fleet::Operations},
};

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

/// Below this speed an aircraft stands, m/s.
const STANDING: f64 = 0.5;

impl Operations {
    /// One tick of ground control: survey, find deadlocks, then clear the
    /// waiting aircraft on as far as the way allows, the most urgent first;
    /// ties by how long they have waited, then tail index.
    pub(in crate::ops::fleet) fn control_ground(&mut self, now: SimTime) {
        self.survey_ground();
        self.find_deadlocks(now);
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
            self.advance(index, now);
        }
        self.requests = requests;
    }

    /// How far along its taxi tail `index` is, if it is on one.
    pub(super) fn along_of(&self, index: usize) -> Option<f64> {
        let motion = self.tails.get(index)?.motion.as_ref()?;
        matches!(
            motion.phase(),
            Phase::Pushback | Phase::TaxiOut | Phase::TaxiIn | Phase::Queue
        )
        .then(|| motion.along())
    }

    /// Where every taxiing aircraft is this tick (on which chains, in which
    /// junctions), and frees the junctions and chains they have left.
    fn survey_ground(&mut self) {
        for airside in self.ground.airports.iter_mut().flatten() {
            airside.taxiing.clear();
            for list in &mut airside.occupied {
                list.clear();
            }
            for list in &mut airside.on_chain {
                list.clear();
            }
        }
        for index in 0..self.taxis.len() {
            let Some(along) = self.along_of(index) else {
                continue;
            };
            let owner = Self::owner(index);
            let Some(taxi) = self.taxis[index].as_mut() else {
                continue;
            };
            let Some(airside) = self.ground.airside_mut(taxi.airport) else {
                continue;
            };
            let tail = along - AIRCRAFT_LENGTH;
            airside.taxiing.push(owner);
            for run in &taxi.runs {
                if run.from <= along
                    && along <= run.to
                    && let Some(list) = airside.on_chain.get_mut(run.chain as usize)
                {
                    list.push(Occupant {
                        tail: owner,
                        position: run.position(along),
                    });
                }
            }
            for (i, passage) in taxi.passages.iter().enumerate() {
                let j = passage.junction as usize;
                if along >= passage.entry - ZONE && tail <= passage.exit + ZONE {
                    if let Some(list) = airside.occupied.get_mut(j) {
                        list.push(owner);
                    }
                } else if i < taxi.next
                    && tail > passage.exit + ZONE
                    && airside.claims.get(j).copied().flatten() == Some(owner)
                {
                    airside.claims[j] = None;
                }
            }
            for k in 0..taxi.locked.min(taxi.runs.len()) {
                let run = taxi.runs[k];
                let later = taxi.runs[k + 1..].iter().any(|r| r.chain == run.chain);
                if run.to < tail && !later {
                    airside.unlock(owner, run.chain);
                }
            }
        }
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
            let at_hold = current != index
                && self.taxis[current].as_ref().is_some_and(|taxi| {
                    taxi.target == Target::Hold
                        && self
                            .along_of(current)
                            .is_some_and(|along| along >= taxi.end - FOLLOW)
                });
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.deadlock_since = if cycle && gives_way {
                    Some(taxi.deadlock_since.unwrap_or(ms(now)))
                } else {
                    None
                };
                taxi.queued = !cycle && at_hold;
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
        let along = self.along_of(index).unwrap_or(taxi.offset);
        let runway = |p: &super::Passage| {
            airside
                .junctions
                .junctions()
                .get(p.junction as usize)
                .is_some_and(|j| j.runway)
        };
        if taxi
            .passages
            .iter()
            .any(|p| runway(p) && along >= p.entry - ZONE && along <= p.exit + ZONE)
        {
            Urgency::OnRunway
        } else if taxi
            .next_hold()
            .is_some_and(|(i, _)| runway(&taxi.passages[i]))
        {
            Urgency::Crossing
        } else if taxi.target == Target::Hold {
            Urgency::Departure
        } else {
            Urgency::Arrival
        }
    }

    /// Whether tail `index` asks to go on, and how urgently: its locks or
    /// its next hold are near.
    fn request(&self, index: usize) -> Option<Urgency> {
        let taxi = self.taxis[index].as_ref()?;
        let motion = self.tails[index].motion.as_ref()?;
        if !matches!(motion.phase(), Phase::TaxiOut | Phase::TaxiIn) || taxi.waiting.is_some() {
            return None;
        }
        let along = motion.along();
        let locks = taxi
            .runs
            .get(taxi.locked)
            .is_some_and(|run| run.from < along + LOOKAHEAD + LOCK_AHEAD / 2.0);
        let hold = taxi
            .next_hold()
            .is_some_and(|(_, hold)| hold - along <= LOOKAHEAD);
        (locks || hold).then(|| self.urgency(index))
    }

    /// Locks tail `index`'s way ahead and, at its hold, clears it into the
    /// junction if it may go.
    fn advance(&mut self, index: usize, now: SimTime) {
        let Some(along) = self.along_of(index) else {
            return;
        };
        self.extend_locks(index, along, now);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        if let Some((i, hold)) = taxi.next_hold()
            && hold - along <= LOOKAHEAD
            && taxi
                .runs
                .get(taxi.locked)
                .is_none_or(|run| run.from >= hold)
        {
            self.enter(index, i, along, now);
        }
    }

    /// Locks the direction of tail `index`'s chains ahead, up to its next
    /// hold or `LOCK_AHEAD`, as far as nobody has locked them the other way.
    fn extend_locks(&mut self, index: usize, along: f64, now: SimTime) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let Some(airside) = self.ground.airside_mut(taxi.airport) else {
            return;
        };
        let horizon = taxi
            .next_hold()
            .map_or((along + LOCK_AHEAD).min(taxi.end), |(_, hold)| hold);
        while let Some(&run) = taxi.runs.get(taxi.locked) {
            if run.from >= horizon {
                break;
            }
            if let Some(other) = airside.opposed(owner, run.chain, run.forward)
                && !taxi.escaped
            {
                taxi.waiting_since.get_or_insert(ms(now));
                taxi.waiting_for = Some(other);
                break;
            }
            airside.lock(owner, run.chain, run.forward);
            taxi.locked += 1;
        }
    }

    /// Clears tail `index`, `along` its path at its hold, into passage `i`
    /// if the junction is free, no through traffic is close, there is room
    /// beyond and nobody comes the other way (or, after waiting too long,
    /// regardless: D75); else it waits, asking to cross a busy runway.
    fn enter(&mut self, index: usize, i: usize, along: f64, now: SimTime) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return;
        };
        let passage = taxi.passages[i];
        let j = passage.junction as usize;
        let escape = taxi.escaped || overdue(taxi, now);
        let other = |slot: Option<u32>| slot.filter(|&o| o != owner);
        let inside = airside
            .occupied
            .get(j)
            .and_then(|list| list.iter().copied().find(|&o| o != owner));
        let beyond = taxi
            .passages
            .iter()
            .skip(i + 1)
            .find_map(|p| p.hold)
            .unwrap_or(passage.exit + LOCK_AHEAD);
        let blocker = inside
            .or_else(|| other(airside.claims.get(j).copied().flatten()))
            .or_else(|| self.through_near(index, passage.junction))
            .or_else(|| self.ahead_on_path(index, passage.exit, passage.exit + ZONE + FOLLOW))
            .or_else(|| {
                taxi.runs
                    .iter()
                    .filter(|run| run.to > passage.exit && run.from < beyond)
                    .find_map(|run| airside.opposed(owner, run.chain, run.forward))
            });
        let runway = airside
            .junctions
            .junctions()
            .get(j)
            .filter(|junction| junction.runway)
            .and_then(|junction| {
                junction
                    .nodes
                    .iter()
                    .find_map(|&n| airside.runway_at(Some(n)))
            });
        let until = time_of(ms(now) + taxi_millis(passage.exit - along + AIRCRAFT_LENGTH));
        let key = runway.map(|runway| RunwayKey {
            airport: taxi.airport,
            runway,
        });
        let runway_busy =
            !escape && key.is_some_and(|key| !self.control.may_cross(key, now, until));
        let in_way = !escape && blocker.is_some();
        if in_way || runway_busy {
            if let Some(key) = key.filter(|_| runway_busy) {
                self.hold_short(index, key, now);
            }
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.waiting_since.get_or_insert(ms(now));
                taxi.waiting_for = blocker.filter(|_| in_way);
            }
            return;
        }
        if let Some(key) = key {
            self.control.crossing(key, until);
        }
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let airport = taxi.airport;
        if let Some(since) = taxi.waiting_since.take() {
            self.stats
                .waited_for_clearance(airport, SimDuration::from_millis(ms(now) - since));
        }
        let regardless = !taxi.escaped && overdue(taxi, now);
        if let Some(airside) = self.ground.airside_mut(airport) {
            if let Some(slot) = airside.claims.get_mut(j) {
                slot.get_or_insert(owner);
            }
            for run in taxi
                .runs
                .iter()
                .skip(taxi.locked)
                .take_while(|run| run.from < beyond)
            {
                airside.lock(owner, run.chain, run.forward);
            }
        }
        taxi.locked = taxi
            .runs
            .iter()
            .position(|run| run.from >= beyond)
            .unwrap_or(taxi.runs.len())
            .max(taxi.locked);
        taxi.next = i + 1;
        taxi.waiting_for = None;
        taxi.deadlock_since = None;
        taxi.crossing = None;
        if regardless {
            taxi.escaped = true;
            self.stats.ground_conflict(airport);
        }
    }

    /// Another aircraft on the through taxiway (or already cleared in) that
    /// is about to pass junction `junction` on tail `index`'s airport: one
    /// that moves within `GAP` of it, or stands right at it.
    pub(super) fn through_near(&self, index: usize, junction: u32) -> Option<u32> {
        let airport = self.taxis[index].as_ref()?.airport;
        let airside = self.ground.airside(airport)?;
        airside.taxiing.iter().find_map(|&other| {
            let k = other as usize;
            let taxi = self.taxis.get(k)?.as_ref().filter(|_| k != index)?;
            let along = self.along_of(k)?;
            let moving = self.tails[k]
                .motion
                .as_ref()
                .is_some_and(|m| m.speed() > STANDING);
            taxi.passages.iter().enumerate().find_map(|(i, p)| {
                let through = p.hold.is_none() || i < taxi.next;
                let distance = p.entry - along;
                (p.junction == junction
                    && through
                    && along < p.exit + ZONE
                    && distance <= GAP
                    && (moving || distance <= FOLLOW + ZONE))
                    .then_some(other)
            })
        })
    }

    /// Another aircraft on tail `index`'s path between `from` and `to`
    /// metres along it, the nearest first.
    pub(super) fn ahead_on_path(&self, index: usize, from: f64, to: f64) -> Option<u32> {
        self.nearest_ahead(index, from, to).map(|(_, tail)| tail)
    }

    /// The nearest other aircraft on tail `index`'s path between `from` and
    /// `to` metres along it, and where along it.
    pub(super) fn nearest_ahead(&self, index: usize, from: f64, to: f64) -> Option<(f64, u32)> {
        let owner = Self::owner(index);
        let taxi = self.taxis[index].as_ref()?;
        let airside = self.ground.airside(taxi.airport)?;
        let mut nearest: Option<(f64, u32)> = None;
        for run in taxi.runs.iter().filter(|r| r.to > from && r.from < to) {
            for occupant in airside
                .on_chain
                .get(run.chain as usize)
                .into_iter()
                .flatten()
            {
                let at = run.along(occupant.position);
                if occupant.tail != owner
                    && at >= from.max(run.from)
                    && at <= to.min(run.to)
                    && nearest.is_none_or(|(d, _)| at < d)
                {
                    nearest = Some((at, occupant.tail));
                }
            }
        }
        nearest
    }

    /// Whether tail `index`'s way from `from` to `to` metres along its path
    /// is clear: nobody on it, or within `PUSH_CLEAR` of `from` on the same
    /// chain either way, nobody in a junction there, and nobody coming the
    /// other way up to its first hold; if so its taxiways are locked (D93).
    /// For a pushback (`from` where it ends) and a landing vacating.
    pub(super) fn clear_way(&mut self, index: usize, from: f64, to: f64) -> bool {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return false;
        };
        if taxi.escaped {
            return true;
        }
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return false;
        };
        let lane = taxi
            .runs
            .iter()
            .find(|run| run.from <= from + 1e-6 && from <= run.to)
            .is_some_and(|run| {
                let here = run.position(from);
                airside
                    .on_chain
                    .get(run.chain as usize)
                    .into_iter()
                    .flatten()
                    .any(|o| o.tail != owner && (o.position - here).abs() < PUSH_CLEAR)
            });
        let junction = taxi.passages.iter().any(|p| {
            p.entry - ZONE <= to
                && p.exit + ZONE >= from
                && (airside
                    .occupied
                    .get(p.junction as usize)
                    .is_some_and(|list| list.iter().any(|&o| o != owner))
                    || airside
                        .claims
                        .get(p.junction as usize)
                        .copied()
                        .flatten()
                        .is_some_and(|o| o != owner))
        });
        let horizon = taxi.next_hold().map_or(taxi.end, |(_, hold)| hold);
        let opposed = taxi
            .runs
            .iter()
            .filter(|run| run.from < horizon)
            .any(|run| airside.opposed(owner, run.chain, run.forward).is_some());
        if lane || junction || opposed || self.ahead_on_path(index, from, to).is_some() {
            return false;
        }
        let Some(taxi) = self.taxis[index].as_mut() else {
            return false;
        };
        if let Some(airside) = self.ground.airside_mut(taxi.airport) {
            for run in taxi.runs.iter().take_while(|run| run.from < horizon) {
                airside.lock(owner, run.chain, run.forward);
                taxi.locked += 1;
            }
        }
        true
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
pub(super) fn overdue(taxi: &Taxi, now: SimTime) -> bool {
    taxi.deadlock_since
        .is_some_and(|since| ms(now) - since >= DEADLOCK_AFTER)
        || (!taxi.queued
            && taxi
                .waiting_since
                .is_some_and(|since| ms(now) - since >= STUCK_AFTER))
}
