//! Stopping on an edge (plan M4 D70) until the node ahead is clear:
//! reservations keep plans apart, this keeps aircraft apart when one ahead
//! stays longer than it planned.

use super::{ESCAPE_AFTER, ms};
use crate::{
    SimTime,
    ground::reserve::{TimedRoute, along_at},
    ops::fleet::Operations,
};

impl Operations {
    /// Stops tail `index` on an edge while someone else is at the node
    /// ahead, and goes on, its plan shifted by the stop, once they have left;
    /// `true` while stopped. Aircraft otherwise check the way only at nodes,
    /// and one ahead may stay longer than it planned.
    pub(super) fn pause(&mut self, index: usize, now: SimTime) -> bool {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_ref() else {
            return false;
        };
        if taxi.escaped {
            return false;
        }
        let at = taxi.departed;
        let Some(ahead) = taxi.route.steps.get(at).copied() else {
            return false;
        };
        let on_edge = taxi.paused.is_some() || (at > 0 && ms(now) < ahead.arrive);
        let waiting = on_edge
            && self
                .ground
                .airside(taxi.airport)
                .is_some_and(|airside| airside.table.occupied(ahead.node, ms(now), owner));
        let (airport, paused) = (taxi.airport, taxi.paused);
        match (taxi.paused, waiting) {
            (None, false) => false,
            (None, true) => {
                if let Some(taxi) = self.taxis[index].as_mut() {
                    // A breakpoint where it stops keeps the shifted plan
                    // continuous.
                    let point = (ms(now), along_at(&taxi.schedule, ms(now)));
                    let slot = taxi.schedule.partition_point(|(t, _)| *t < point.0);
                    taxi.schedule.insert(slot, point);
                    taxi.paused = Some(ms(now));
                }
                true
            }
            (Some(_), true) => {
                // Stopped this long, it is stuck in a deadlock (D75).
                if paused.is_some_and(|since| ms(now) - since >= ESCAPE_AFTER) {
                    self.resume(index, now);
                    self.stats.ground_conflict(airport);
                    self.mark_escaped(index);
                    return false;
                }
                true
            }
            (Some(_), false) => {
                self.resume(index, now);
                false
            }
        }
    }

    /// Ends tail `index`'s stop on an edge: the rest of its plan moves on by
    /// the time stopped and is booked again.
    fn resume(&mut self, index: usize, now: SimTime) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        let Some(since) = taxi.paused.take() else {
            return;
        };
        let shift = ms(now) - since;
        for step in taxi.route.steps.iter_mut().skip(taxi.departed) {
            step.arrive += shift;
            step.leave += shift;
        }
        for point in taxi.schedule.iter_mut().filter(|(t, _)| *t > since) {
            point.0 += shift;
        }
        let rest = TimedRoute {
            steps: taxi
                .route
                .steps
                .get(taxi.departed.saturating_sub(1)..)
                .map_or_else(Vec::new, <[_]>::to_vec),
            open: taxi.route.open,
        };
        let airport = taxi.airport;
        if let Some(airside) = self.ground.airside_mut(airport) {
            airside.table.release(owner);
            airside.table.reserve(&rest, owner);
        }
    }
}
