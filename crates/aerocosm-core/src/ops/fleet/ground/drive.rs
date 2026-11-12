//! Taxiing as pilots do (plan M4 D93): each tick an aircraft moves on
//! towards the nearest of its next hold, the edge of a junction someone
//! else is in, a following distance behind the aircraft ahead, the end of
//! the taxiways it has locked, and its route's end; then it finishes at its
//! stand, waiting spot or holding position.

use super::{AIRCRAFT_LENGTH, FOLLOW, Target, ZONE, ms};
use crate::{
    SimDuration, SimTime, TICK,
    ground::TAXI_SPEED,
    ops::{
        fleet::Operations,
        motion::{Event, Motion},
    },
    scenario::Scenario,
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
        self.go_around(index, along);
        let stop = self.stop(index, along);
        let dt = TICK.as_secs_f64();
        let next = (along + TAXI_SPEED * dt).min(stop.at.max(along));
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
                .map_or(run.from, |p| p.entry - ZONE);
            stop = stop.nearer(short, taxi.waiting_for, false);
        }
        // Short of a junction someone else is in or has been cleared into.
        for passage in taxi
            .passages
            .iter()
            .filter(|p| p.entry - ZONE > along && p.entry - ZONE < stop.at)
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
                stop = stop.nearer(passage.entry - ZONE, other, false);
                break;
            }
        }
        // Behind the aircraft ahead.
        if let Some((at, tail)) = self.nearest_ahead(index, along + 1e-3, stop.at + FOLLOW) {
            stop = stop.nearer(at - FOLLOW, Some(tail), true);
        }
        stop
    }

    /// A landing that rolled out at an exit another aircraft is still at
    /// would have been sent around by the tower; go-arounds come later
    /// (plan M4 D90), so it taxies on regardless (D75).
    fn go_around(&mut self, index: usize, along: f64) {
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        if taxi.escaped || taxi.target == Target::Hold || along > taxi.offset + 1.0 {
            return;
        }
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return;
        };
        let owner = Self::owner(index);
        let on_runway = taxi.passages.first().is_some_and(|p| {
            p.entry <= taxi.offset + 1e-6
                && airside
                    .junctions
                    .junctions()
                    .get(p.junction as usize)
                    .is_some_and(|j| j.runway)
        });
        let blocked = on_runway
            && (self
                .nearest_ahead(index, along, along + AIRCRAFT_LENGTH)
                .is_some()
                || taxi.passages.first().is_some_and(|p| {
                    airside
                        .occupied
                        .get(p.junction as usize)
                        .is_some_and(|list| {
                            list.iter().any(|&o| {
                                o != owner
                                    && self
                                        .taxis
                                        .get(o as usize)
                                        .and_then(Option::as_ref)
                                        .is_some_and(|t| t.end_node.is_some() && t.offset <= 1e-6)
                                    && self.along_of_tail(o).is_some_and(|a| a < AIRCRAFT_LENGTH)
                            })
                        })
                }));
        if blocked && let Some(taxi) = self.taxis[index].as_mut() {
            taxi.escaped = true;
            let airport = taxi.airport;
            self.stats.ground_conflict(airport);
        }
    }

    /// How far along its taxi tail `tail` is.
    fn along_of_tail(&self, tail: u32) -> Option<f64> {
        self.tails
            .get(tail as usize)?
            .motion
            .as_ref()
            .map(Motion::along)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
