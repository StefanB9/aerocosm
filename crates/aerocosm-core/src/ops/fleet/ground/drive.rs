//! Taxiing as pilots do (plan M4 D93): each tick an aircraft moves on
//! towards the nearest of its next hold, the edge of a junction someone
//! else is in, a following distance behind the aircraft ahead, the end of
//! the taxiways it has locked, and its route's end; then it finishes at its
//! stand, waiting spot or holding position.

use super::{EDGE, FOLLOW, HOLD_SHORT, STALLED_AFTER, Target, control::overdue, ms};
use crate::{
    LatLon, SimDuration, SimTime, TICK,
    ground::junctions::Passage,
    ops::{
        fleet::Operations,
        motion::{Event, Motion},
    },
    scenario::Scenario,
    units::Degrees,
    world::codes::AirlineIcao,
};

/// What stops an aircraft where.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stop {
    /// How far along its path it may taxi, m.
    at: f64,
    /// The aircraft in the way, if one is.
    by: Option<u32>,
    /// Whether that aircraft is one it follows.
    following: bool,
}

impl Stop {
    /// The nearer of `self` and a stop `at`, because of `by`.
    fn nearer(self, at: f64, by: Option<u32>, following: bool) -> Self {
        if at < self.at {
            Self { at, by, following }
        } else {
            self
        }
    }
}

impl Operations {
    /// Moves a tail on a controlled taxi for one tick; `Some(Arrived)` when
    /// it reaches its stand.
    pub(in crate::ops::fleet) fn drive_taxi(
        &mut self,
        scenario: &Scenario,
        index: usize,
        motion: &mut Motion,
        now: SimTime,
        airline: AirlineIcao,
    ) -> Option<Event> {
        self.release_exit(index);
        let taxi = self.taxis[index].as_ref()?;
        if taxi.waiting.is_some() {
            if self.wait_for_stand(scenario, index, now, airline) {
                // Deplaned at the spot, towed to remote parking (D127).
                motion.arrive_now();
                return Some(Event::Arrived);
            }
            motion.taxi_at(motion.along(), 0.0);
            return None;
        }
        let along = motion.along();
        // Held up far too long, or the one to give way in a deadlock: it is
        // sent another way (D93), or if there is none taxies regardless
        // (D75).
        if self.taxis[index]
            .as_ref()
            .is_some_and(|taxi| !taxi.escaped && overdue(taxi, now))
            && !self.reroute(scenario, index, motion, now)
            && let Some(taxi) = self.taxis.get_mut(index)
        {
            taxi.escaped = true;
            let airport = taxi.airport;
            self.stats.ground_conflict(airport);
        }
        let mut stop = self.stop(index, along);
        let dt = TICK.as_secs_f64();
        let speed = self.taxis[index].as_ref().map_or(0.0, |taxi| {
            taxi.speeds.next_speed(along, motion.speed(), stop.at)
        });
        let reach = (stop.at - along).clamp(0.0, speed * dt);
        let mut sighted = false;
        if reach > 0.0
            && let Some(other) = self.seen_ahead(index, motion, reach)
        {
            stop = stop.nearer(along, Some(other), false);
            sighted = true;
        }
        let next = (along + reach).min(stop.at.max(along));
        self.claim_entered(index, along, next);
        motion.taxi_at(next, (next - along) / dt);
        if next - along > 1e-3 {
            self.seen_moving(index, motion);
        }
        let taxi = self.taxis.get_mut(index)?;
        taxi.limit = stop.at.max(along);
        taxi.sighted = sighted;
        taxi.leader = stop.by.filter(|_| stop.following);
        if next - along < 1e-3 && stop.by.is_some() {
            taxi.waiting_since.get_or_insert(ms(now));
            taxi.waiting_for = stop.by;
        } else if next - along >= 1e-3 && taxi.next_hold().is_none_or(|(_, h)| h > next) {
            taxi.waiting_for = None;
            if taxi.crossing.is_none() {
                taxi.waiting_since = None;
            }
        }
        let idle = next - along < 1e-3 && taxi.waiting_since.is_none() && next < taxi.end - 1e-6;
        let stalled = stalls(&mut taxi.idle_since, idle, ms(now));
        if next < taxi.end - 1e-6 {
            if stalled {
                let airport = taxi.airport;
                self.stats.stalled(airport);
            }
            return None;
        }
        let end = ms(now) + TICK.as_millis();
        let (airport, target, took) = (taxi.airport, taxi.target, end - taxi.started);
        let delay = took - taxi.unimpeded;
        self.stats.taxied(
            airport,
            SimDuration::from_millis(delay.max(0)),
            SimDuration::from_millis(took),
        );
        match target {
            Target::Hold => {
                // The taxi stays on record until line-up: it still holds
                // the holding position.
                motion.queue_now();
                if let Some(taxi) = self.taxis.get_mut(index) {
                    taxi.queued_at = Some(end);
                }
                None
            }
            Target::Stand(_) => {
                motion.arrive_now();
                self.ground.release(airport, Self::owner(index));
                self.taxis.clear(index);
                Some(Event::Arrived)
            }
            Target::Spot(_) => {
                if let Some(taxi) = self.taxis.get_mut(index) {
                    taxi.waiting = Some(end);
                    taxi.spot_since = end;
                }
                None
            }
        }
    }

    /// Where tail `index`, `along` its path, has to stop, and for whom.
    fn stop(&self, index: usize, along: f64) -> Stop {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return Stop {
                at: along,
                by: None,
                following: false,
            };
        };
        let mut stop = Stop {
            at: taxi.end,
            by: None,
            following: false,
        };
        if taxi.escaped {
            return stop;
        }
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return stop;
        };
        if let Some((_, hold)) = taxi.next_hold() {
            stop = stop.nearer(hold, None, false);
        }
        // Short of the junction before the taxiways it has not locked.
        if let Some(run) = taxi.runs.get(taxi.locked) {
            let short = super::control::short_of_unlocked(&taxi.passages, run.from);
            stop = stop.nearer(short, taxi.waiting_for, false);
        }
        // Short of a junction someone else is in or has been cleared into.
        for passage in taxi
            .passages
            .iter()
            // Stopped right at a zone's edge it is still outside.
            .filter(|p| p.entry - HOLD_SHORT > along - EDGE && p.entry - HOLD_SHORT < stop.at)
        {
            let j = passage.junction as usize;
            let other = airside
                .occupied
                .get(j)
                .and_then(|list| list.iter().copied().find(|&o| o != owner))
                .or_else(|| {
                    airside
                        .claims
                        .get(j)
                        .copied()
                        .flatten()
                        .filter(|&o| o != owner)
                });
            if other.is_some() {
                stop = stop.nearer(passage.entry - HOLD_SHORT, other, false);
                break;
            }
        }
        // Behind the aircraft ahead.
        if let Some((at, tail)) = self.nearest_ahead(index, along + 1e-3, stop.at + FOLLOW) {
            stop = stop.nearer(at - FOLLOW, Some(tail), true);
        }
        stop
    }

    /// Tail `index` has moved: those who move after it this tick see it
    /// where it is now.
    fn seen_moving(&mut self, index: usize, motion: &Motion) {
        let Some((at, heading)) = self
            .paths
            .get(index)
            .and_then(Option::as_ref)
            .map(|path| path.locate(motion))
        else {
            return;
        };
        let Some(airport) = self.taxis[index].as_ref().map(|taxi| taxi.airport) else {
            return;
        };
        if let Some(airside) = self.ground.airside_mut(airport) {
            moved(&mut airside.seen, Self::owner(index), at, heading, true);
        }
    }

    /// Tail `index`, moving from `along` to `next`, claims the junctions
    /// whose zone it enters, so that nobody else enters them this tick.
    fn claim_entered(&mut self, index: usize, along: f64, next: f64) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        let Some(airside) = self.ground.airside_mut(taxi.airport) else {
            return;
        };
        for (i, passage) in taxi.passages.iter().enumerate() {
            // Not a junction it has yet to be cleared into from its hold.
            if i >= taxi.next && passage.hold.is_some() {
                continue;
            }
            let edge = passage.entry - HOLD_SHORT;
            if edge > along - EDGE
                && edge <= next
                && let Some(slot) = airside.claims.get_mut(passage.junction as usize)
            {
                slot.get_or_insert(owner);
            }
        }
    }

    /// Another aircraft tail `index` would close in on by moving `step`
    /// metres now (see and avoid), unless it taxies regardless.
    fn seen_ahead(&self, index: usize, motion: &Motion, step: f64) -> Option<u32> {
        let owner = Self::owner(index);
        let taxi = self.taxis[index].as_ref().filter(|t| !t.escaped)?;
        let path = self.paths.get(index)?.as_ref()?;
        let (here, _) = path.locate(motion);
        // Where the move ends along the taxi, through its turns: a straight
        // probe along the heading misses one the curve turns towards.
        let out = motion.phase() == crate::ops::Phase::TaxiOut;
        let next = path.taxi_point(out, motion.along() + step);
        let airside = self.ground.airside(taxi.airport)?;
        // Nobody farther than this can be in the way: a cheap box first,
        // great circles only inside it.
        let reach = (SEE_AND_AVOID + step + 10.0) / METRES_PER_DEGREE;
        let (lat, lon) = (here.latitude().value(), here.longitude().value());
        let wide = reach / lat.to_radians().cos().max(0.1);
        airside
            .seen
            .iter()
            .filter(|other| {
                (other.at.latitude().value() - lat).abs() <= reach
                    && (other.at.longitude().value() - lon).abs() <= wide
            })
            .find(|other| other.tail != owner && in_the_way(here, next, other, owner))
            .map(|other| other.tail)
    }
}

/// Keeps since when an aircraft has stood `idle` (with nobody to wait for)
/// in `since`; true at the moment it has done so for `STALLED_AFTER`.
fn stalls(since: &mut Option<i64>, idle: bool, now: i64) -> bool {
    if !idle {
        *since = None;
        return false;
    }
    now - *since.get_or_insert(now) == STALLED_AFTER
}

/// Metres per degree of latitude (and of longitude at the equator).
const METRES_PER_DEGREE: f64 = 111_195.0;
/// Pilots do not close in on another aircraft in front nearer than this,
/// m, …
const SEE_AND_AVOID: f64 = 50.0;
/// …in front meaning ahead and at most this far to either side of their
/// way, m…
const IN_FRONT: f64 = 25.0;
/// …nor on anyone nearer than this, m: an aircraft holding short of a
/// junction is passed wingtip-clear.
const TOO_CLOSE: f64 = 25.0;
/// An aircraft waiting at a hold line, drawn wingtip-clear of the way, is
/// passed down to this, m: where lines are drawn nearer the way than they
/// should be, the two would otherwise wait for each other (plan M6 D153).
/// A metre more than the least distance the ground test allows: two that
/// move in the same tick each judge the other where it was.
const PAST_HOLD: f64 = 21.0;

/// Another aircraft as a pilot sees it: where it is, which way it heads and
/// whether it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Seen {
    pub(super) tail: u32,
    pub(super) at: LatLon,
    pub(super) heading: Degrees,
    pub(super) moving: bool,
    /// Waiting at a hold line, which is drawn wingtip-clear of the way.
    pub(super) at_hold: bool,
    /// The aircraft it stands for, if any.
    pub(super) waits_for: Option<u32>,
    /// Whether it stands for that aircraft because it sees it in its way,
    /// not for a clearance.
    pub(super) by_sight: bool,
}

/// Moves `owner`'s own sighting in `seen` to where it now is, so that those
/// who move after it in the same tick judge it there, not where it stood
/// at the survey (plan M7 PR 1). Its other sightings (the rest of a
/// pushback) stay.
pub(super) fn moved(seen: &mut [Seen], owner: u32, at: LatLon, heading: Degrees, moving: bool) {
    if let Some(own) = seen.iter_mut().find(|s| s.tail == owner) {
        own.at = at;
        own.heading = heading;
        own.moving = moving;
        own.at_hold &= !moving;
    }
}

/// Whether an aircraft `along` its path through `passages`, cleared up to
/// passage `next`, stands at a line drawn wingtip-clear of crossing traffic:
/// the hold before a passage it still needs clearance for, or short of a
/// junction.
pub(super) fn at_a_line(passages: &[Passage], next: usize, along: f64) -> bool {
    passages.iter().enumerate().any(|(i, p)| {
        let at = |line: f64| (along - line).abs() <= 1.0;
        at(p.entry - HOLD_SHORT) || (i >= next && p.hold.is_some_and(at))
    })
}

/// Whether an aircraft at `here`, about to move to `next`, has to stop for
/// `other` (see and avoid): the move brings it closer, within
/// `SEE_AND_AVOID` of one in front on its way or `TOO_CLOSE` of any. When both
/// close in on each other, or each stands for the sight of the other, the
/// one with priority (the lower tail) goes on; one that stands for tail
/// `me`'s clearance is passed in any case; all down to `PAST_HOLD`.
fn in_the_way(here: LatLon, next: LatLon, other: &Seen, me: u32) -> bool {
    let first = me < other.tail;
    let before = here.distance_to(other.at).value();
    let ahead = here.initial_bearing_to(other.at).value();
    let going = here.initial_bearing_to(next).value();
    let off_nose = ((ahead - going + 540.0).rem_euclid(360.0) - 180.0).to_radians();
    // The nearest the move comes, not only where it ends: at full taxi
    // speed a step can pass right over another aircraft (plan M6 D154).
    let step = here.distance_to(next).value();
    let along = (before * off_nose.cos()).clamp(0.0, step);
    let nearest = (before * before + along * along - 2.0 * before * along * off_nose.cos())
        .max(0.0)
        .sqrt();
    let after = next.distance_to(other.at).value().min(nearest);
    if after >= before {
        return false;
    }
    if other.at_hold {
        return after < PAST_HOLD;
    }
    if after >= SEE_AND_AVOID {
        return false;
    }
    let aside = before * off_nose.sin().abs();
    if after >= TOO_CLOSE && (off_nose.cos() <= 0.0 || aside > IN_FRONT) {
        return false;
    }
    let towards = other.at.initial_bearing_to(here).value();
    let off = (towards - other.heading.value() + 540.0).rem_euclid(360.0) - 180.0;
    let closing = other.moving && off.abs() < 90.0;
    let waits = !other.moving && other.waits_for == Some(me);
    // It waits for this one's clearance: it cannot go first, so this one
    // does. Each standing for the sight of the other, or closing in on each
    // other: the one with priority.
    let goes = (waits && !other.by_sight) || ((closing || waits) && first);
    !(goes && after >= PAST_HOLD)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_pilots_do_not_close_in_on_others() -> Result<()> {
        let metres_north = |m: f64| LatLon::new(50.03 + m / 111_195.0, 8.56);
        let (here, next) = (metres_north(0.0)?, metres_north(5.0)?);
        let standing = |m: f64| -> Result<Seen> {
            Ok(Seen {
                tail: 1,
                at: metres_north(m)?,
                heading: Degrees::from_raw(0.0),
                moving: false,
                at_hold: false,
                waits_for: None,
                by_sight: false,
            })
        };
        // Standing 40 m ahead: stop; 80 m ahead: go on.
        assert!(in_the_way(here, next, &standing(40.0)?, 0));
        assert!(!in_the_way(here, next, &standing(80.0)?, 0));
        // Behind: moving away from it is fine.
        assert!(!in_the_way(here, next, &standing(-30.0)?, 2));
        // Holding short 45 m off to the side of the way: pass it.
        let east = |m: f64| LatLon::new(50.03 + 20.0 / 111_195.0, 8.56 + m / 71_470.0);
        let aside = Seen {
            at: east(42.0)?,
            ..standing(0.0)?
        };
        assert!(!in_the_way(here, next, &aside, 2));
        // Holding short at a side branch ahead, 45 m along and 26 m off the
        // way: pass it.
        let side = Seen {
            at: LatLon::new(50.03 + 45.0 / 111_195.0, 8.56 + 26.0 / 71_470.0)?,
            ..standing(0.0)?
        };
        assert!(!in_the_way(here, next, &side, 2));
        // Anyone closer than 25 m that it closes in on: stop.
        let close = Seen {
            at: east(15.0)?,
            ..standing(0.0)?
        };
        assert!(in_the_way(here, next, &close, 2));
        // Unless it waits at a hold line, drawn wingtip-clear: those are
        // passed down to half an airliner's length…
        let at_hold = Seen {
            at: east(18.0)?,
            at_hold: true,
            ..close
        };
        assert!(!in_the_way(here, next, &at_hold, 2));
        // 22 m from the way at its line: still passed (plan M6 D153).
        let near_line = Seen {
            at: LatLon::new(50.03 + 5.0 / 111_195.0, 8.56 + 22.0 / 71_470.0)?,
            at_hold: true,
            ..close
        };
        assert!(!in_the_way(here, next, &near_line, 2));
        // …but never driven into.
        let on_the_way = Seen {
            at_hold: true,
            ..standing(20.0)?
        };
        assert!(in_the_way(here, next, &on_the_way, 2));
        // Standing in front and waiting for this one in turn: the one with
        // priority goes on, but not closer than half an airliner's length.
        let stand_off = |m: f64, me: u32| -> Result<Seen> {
            Ok(Seen {
                waits_for: Some(me),
                by_sight: true,
                ..standing(m)?
            })
        };
        assert!(!in_the_way(here, next, &stand_off(40.0, 0)?, 0));
        assert!(in_the_way(here, next, &stand_off(40.0, 2)?, 2));
        assert!(in_the_way(here, next, &stand_off(19.0, 0)?, 0));
        // One that waits for this one's clearance, not for the sight of
        // it, is passed whoever has priority: it cannot go first.
        let waiting = Seen {
            by_sight: false,
            ..stand_off(40.0, 2)?
        };
        assert!(!in_the_way(here, next, &waiting, 2));
        // Head-on, both moving: the one with priority goes on.
        let coming = Seen {
            heading: Degrees::from_raw(180.0),
            moving: true,
            ..standing(40.0)?
        };
        assert!(!in_the_way(here, next, &coming, 0));
        assert!(in_the_way(here, next, &coming, 2));
        // But not to closer than 21 m.
        let close_head_on = Seen {
            at: metres_north(24.0)?,
            ..coming
        };
        assert!(in_the_way(here, next, &close_head_on, 0));
        // A full-speed step never jumps over anyone: 7.5 m ahead on the
        // way, a 15 m move ends as far beyond (plan M6 D154).
        let long_step = metres_north(15.0)?;
        assert!(in_the_way(here, long_step, &standing(7.5)?, 2));
        let jumped_hold = Seen {
            at_hold: true,
            ..standing(7.5)?
        };
        assert!(in_the_way(here, long_step, &jumped_hold, 2));
        Ok(())
    }

    #[test]
    fn test_a_move_is_seen_at_once() -> Result<()> {
        let at = |m: f64| LatLon::new(50.03 + m / 111_195.0, 8.56);
        let sighting = |tail: u32, m: f64| -> Result<Seen> {
            Ok(Seen {
                tail,
                at: at(m)?,
                heading: Degrees::from_raw(0.0),
                moving: false,
                at_hold: true,
                waits_for: None,
                by_sight: false,
            })
        };
        let mut seen = [sighting(1, 0.0)?, sighting(2, 50.0)?, sighting(1, 10.0)?];
        moved(&mut seen, 1, at(1.0)?, Degrees::from_raw(0.0), true);
        assert!((seen[0].at.distance_to(at(1.0)?).value()).abs() < 1e-6);
        assert!(seen[0].moving && !seen[0].at_hold);
        assert_eq!(seen[1], sighting(2, 50.0)?, "nobody else's");
        assert_eq!(seen[2], sighting(1, 10.0)?, "only its first sighting");
        Ok(())
    }

    #[test]
    fn test_aircraft_stand_at_hold_lines_and_short_of_junctions() {
        let passages = [
            Passage {
                junction: 0,
                entry: 300.0,
                exit: 330.0,
                hold: None,
            },
            Passage {
                junction: 1,
                entry: 800.0,
                exit: 850.0,
                hold: Some(760.0),
            },
        ];
        // Short of the first junction, at the hold of the second.
        assert!(at_a_line(&passages, 0, 300.0 - HOLD_SHORT));
        assert!(at_a_line(&passages, 1, 760.5));
        // Anywhere else, or at a hold already cleared.
        assert!(!at_a_line(&passages, 0, 500.0));
        assert!(!at_a_line(&passages, 2, 760.0));
    }

    #[test]
    fn test_a_stall_is_counted_once_after_five_idle_minutes() {
        let mut since = None;
        let mut counted = 0;
        for second in 0..1_000 {
            counted += u32::from(stalls(&mut since, true, second * 1_000));
        }
        assert_eq!(counted, 1);
        assert_eq!(since, Some(0));
        // Moving or waiting for someone starts it afresh.
        assert!(!stalls(&mut since, false, 1_000_000));
        assert_eq!(since, None);
        assert!(!stalls(&mut since, true, 1_001_000));
        assert!(!stalls(&mut since, true, 1_300_000));
        assert!(stalls(&mut since, true, 1_301_000));
    }

    #[test]
    fn test_the_nearest_stop_wins() {
        let stop = Stop {
            at: 500.0,
            by: None,
            following: false,
        };
        let held = stop.nearer(300.0, Some(4), false);
        assert_eq!(held.by, Some(4));
        assert!((held.at - 300.0).abs() < 1e-9);
        let behind = held.nearer(240.0, Some(7), true);
        assert!(behind.following && behind.by == Some(7));
        assert_eq!(behind.nearer(400.0, Some(9), false), behind);
    }
}
