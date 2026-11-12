//! Taxiing as pilots do (plan M4 D93): each tick an aircraft moves on
//! towards the nearest of its next hold, the edge of a junction someone
//! else is in, a following distance behind the aircraft ahead, the end of
//! the taxiways it has locked, and its route's end; then it finishes at its
//! stand, waiting spot or holding position.

use super::{EDGE, FOLLOW, HOLD_SHORT, Target, control::overdue, ms};
use crate::{
    LatLon, SimDuration, SimTime, TICK,
    ground::{TAXI_SPEED, junctions::Passage},
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
        let taxi = self.taxis[index].as_ref()?;
        if taxi.waiting.is_some() {
            self.wait_for_stand(scenario, index, now, airline);
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
            && let Some(taxi) = self.taxis[index].as_mut()
        {
            taxi.escaped = true;
            let airport = taxi.airport;
            self.stats.ground_conflict(airport);
        }
        let mut stop = self.stop(index, along);
        let dt = TICK.as_secs_f64();
        let reach = (stop.at - along).clamp(0.0, TAXI_SPEED * dt);
        if reach > 0.0
            && let Some(other) = self.seen_ahead(index, motion, reach)
        {
            stop = stop.nearer(along, Some(other), false);
        }
        let next = (along + TAXI_SPEED * dt).min(stop.at.max(along));
        self.claim_entered(index, along, next);
        motion.taxi_at(next, (next - along) / dt);
        let taxi = self.taxis[index].as_mut()?;
        taxi.limit = stop.at.max(along);
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
        if next < taxi.end - 1e-6 {
            return None;
        }
        let end = ms(now) + TICK.as_millis();
        let (airport, target, delay) = (
            taxi.airport,
            taxi.target,
            end - taxi.started - taxi.unimpeded,
        );
        self.stats
            .taxied(airport, SimDuration::from_millis(delay.max(0)));
        match target {
            Target::Hold => {
                // The taxi stays on record until line-up: it still holds
                // the holding position.
                motion.queue_now();
                None
            }
            Target::Stand(_) => {
                motion.arrive_now();
                self.ground.release(airport, Self::owner(index));
                self.taxis[index] = None;
                Some(Event::Arrived)
            }
            Target::Spot(_) => {
                if let Some(taxi) = self.taxis[index].as_mut() {
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
            let short = taxi
                .passages
                .iter()
                .rev()
                .find(|p| p.entry <= run.from + 1e-6)
                .map_or(run.from, |p| p.entry - HOLD_SHORT);
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
        let (here, heading) = self.paths.get(index)?.as_ref()?.locate(motion);
        let next = here.destination(heading, crate::units::Meters::from_raw(step));
        let airside = self.ground.airside(taxi.airport)?;
        airside
            .seen
            .iter()
            .find(|other| other.tail != owner && in_the_way(here, next, other, owner))
            .map(|other| other.tail)
    }
}

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
/// passed down to this, m: half an airliner's length.
const PAST_HOLD: f64 = 22.5;

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
/// close in on each other, or the other stands for tail `me`, the one with
/// priority (the lower tail) goes on; past a standing one down to
/// `PAST_HOLD`.
fn in_the_way(here: LatLon, next: LatLon, other: &Seen, me: u32) -> bool {
    let first = me < other.tail;
    let (before, after) = (
        here.distance_to(other.at).value(),
        next.distance_to(other.at).value(),
    );
    if after >= before {
        return false;
    }
    if other.at_hold {
        return after < PAST_HOLD;
    }
    if after >= SEE_AND_AVOID {
        return false;
    }
    let ahead = here.initial_bearing_to(other.at).value();
    let going = here.initial_bearing_to(next).value();
    let off_nose = ((ahead - going + 540.0).rem_euclid(360.0) - 180.0).to_radians();
    let aside = before * off_nose.sin().abs();
    if after >= TOO_CLOSE && (off_nose.cos() <= 0.0 || aside > IN_FRONT) {
        return false;
    }
    let towards = other.at.initial_bearing_to(here).value();
    let off = (towards - other.heading.value() + 540.0).rem_euclid(360.0) - 180.0;
    let closing = other.moving && off.abs() < 90.0;
    let stand_off = !other.moving && other.waits_for == Some(me);
    !((closing || stand_off) && first && after >= PAST_HOLD)
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
                ..standing(m)?
            })
        };
        assert!(!in_the_way(here, next, &stand_off(40.0, 0)?, 0));
        assert!(in_the_way(here, next, &stand_off(40.0, 2)?, 2));
        assert!(in_the_way(here, next, &stand_off(20.0, 0)?, 0));
        // Head-on, both moving: the one with priority goes on.
        let coming = Seen {
            heading: Degrees::from_raw(180.0),
            moving: true,
            ..standing(40.0)?
        };
        assert!(!in_the_way(here, next, &coming, 0));
        assert!(in_the_way(here, next, &coming, 2));
        // But not to closer than half an airliner's length.
        let close_head_on = Seen {
            at: metres_north(25.0)?,
            ..coming
        };
        assert!(in_the_way(here, next, &close_head_on, 0));
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
