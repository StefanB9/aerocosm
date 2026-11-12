//! Taxiing under clearances (plan M4 D79): each tick an aircraft moves on
//! towards its clearance limit, frees what lies an aircraft length behind
//! it, and finishes at its stand, waiting spot or holding position.

use super::{AIRCRAFT_LENGTH, FOLLOW_DISTANCE, Leader, Target, ms};
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
        self.go_around(index);
        let behind = self.behind_leader(index);
        let taxi = self.taxis[index].as_ref()?;
        let along = motion.along();
        let dt = TICK.as_secs_f64();
        let next = (along + TAXI_SPEED * dt).min(taxi.limit().min(behind).max(along));
        motion.taxi_at(next, (next - along) / dt);
        self.pass_hops(index, next);
        let taxi = self.taxis[index].as_ref()?;
        if taxi.granted < taxi.hops.len() || next < taxi.end - 1e-6 {
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

    /// A landing rolled out at its exit holds it until cleared off the
    /// runway. At an exit someone else holds it would have been sent around
    /// by the tower; go-arounds are not modelled, so it taxies on
    /// regardless (D75).
    fn go_around(&mut self, index: usize) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let Some(airside) = self.ground.airside_mut(taxi.airport) else {
            return;
        };
        let Some(exit) = taxi.hops.first() else {
            return;
        };
        if taxi.granted > 0 || taxi.escaped || airside.runway_at(exit.node).is_none() {
            return;
        }
        if airside.clearances.blocker(owner, exit, None).is_some() {
            taxi.escaped = true;
            self.stats.ground_conflict(taxi.airport);
        } else {
            airside.clearances.enter(owner, exit);
        }
    }

    /// How far tail `index` may taxi behind the aircraft it follows; those
    /// that no longer limit it, it no longer follows.
    fn behind_leader(&mut self, index: usize) -> f64 {
        let (taxis, tails) = (&mut self.taxis, &self.tails);
        let Some(taxi) = taxis[index].as_ref() else {
            return f64::INFINITY;
        };
        let limit = taxi.limit();
        let mut behind = f64::INFINITY;
        let mut keep = taxi.leaders.clone();
        keep.retain(|leader| {
            let at = taxis[leader.tail as usize]
                .as_ref()
                .and(tails[leader.tail as usize].motion.as_ref())
                .map(|motion| follow_limit(leader, motion.along()));
            match at {
                Some(at) if at < limit => {
                    behind = behind.min(at);
                    true
                }
                _ => false,
            }
        });
        if let Some(taxi) = taxis[index].as_mut() {
            taxi.leaders = keep;
        }
        if behind.is_infinite() {
            return behind;
        }
        // Behind its leader it stops only where it blocks no junction, as
        // any clearance limit does.
        let Some(taxi) = self.taxis[index].as_ref() else {
            return behind;
        };
        let Some(airside) = self.ground.airside(taxi.airport) else {
            return behind;
        };
        taxi.hops[..taxi.granted.min(taxi.hops.len())]
            .iter()
            .rev()
            .find(|hop| hop.along <= behind && airside.clearances.stoppable(hop.point))
            .map_or(f64::NEG_INFINITY, |hop| hop.along)
    }

    /// Frees the hops tail `index` has left an aircraft length behind at
    /// `along`, except where it ends: past a holding position it lines up,
    /// and nobody follows it there.
    fn pass_hops(&mut self, index: usize, along: f64) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let Some(airside) = self.ground.airside_mut(taxi.airport) else {
            return;
        };
        while taxi.passed < taxi.granted.min(taxi.hops.len().saturating_sub(1))
            && taxi
                .hops
                .get(taxi.passed)
                .is_some_and(|hop| hop.along + AIRCRAFT_LENGTH < along)
        {
            airside.clearances.leave(owner, &taxi.hops[taxi.passed]);
            taxi.passed += 1;
        }
    }
}

/// Where a follower may taxi up to along its path, with its leader at
/// `leader_along` along the leader's: as far short of the shared point as
/// the leader is, and the following distance behind.
fn follow_limit(leader: &Leader, leader_along: f64) -> f64 {
    leader.mine - (leader.theirs - leader_along) - FOLLOW_DISTANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_follow_limit_keeps_the_following_distance_through_the_shared_point() {
        // The shared point is 500 m along my path and 300 m along the
        // leader's; the leader is at 250 m, 50 m short of it.
        let limit = |leader| {
            follow_limit(
                &Leader {
                    tail: 1,
                    mine: 500.0,
                    theirs: 300.0,
                },
                leader,
            )
        };
        assert!((limit(250.0) - (500.0 - 50.0 - FOLLOW_DISTANCE)).abs() < 1e-9);
        // Past the point by 200 m, I may come up to it and beyond.
        assert!((limit(500.0) - (700.0 - FOLLOW_DISTANCE)).abs() < 1e-9);
    }
}
