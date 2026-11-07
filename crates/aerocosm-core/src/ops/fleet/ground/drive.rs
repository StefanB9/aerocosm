//! Following timed plans tick by tick, holding up and planning again when
//! the way ahead is not free (plan M4 D70, D75, D77).

use super::{ESCAPE_AFTER, Plan, QUEUE_ESTIMATE, RETRY, SPOT_ESTIMATE, Start, Target, ms, time_of};
use crate::{
    Id, SimDuration, SimTime, TICK,
    ground::{
        Node,
        reserve::{NODE_BUFFER, TimedRoute, along_at},
    },
    ops::{
        control::RunwayKey,
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
        let end = ms(now) + TICK.as_millis();
        let taxi = self.taxis[index].as_ref()?;
        if taxi.waiting.is_some() {
            self.wait_for_stand(scenario, index, now, airline);
            motion.taxi_at(motion.along(), 0.0);
            return None;
        }
        if let Some((since, retry)) = taxi.blocked {
            if let Some((key, _)) = taxi.crossing {
                self.hold_short(index, key, now);
            }
            if end >= retry {
                self.replan(scenario, index, now, since);
            }
            motion.taxi_at(motion.along(), 0.0);
            return None;
        }
        if !self.advance_steps(index, end, now) {
            motion.taxi_at(motion.along(), 0.0);
            return None;
        }
        let taxi = self.taxis[index].as_ref()?;
        let along = taxi.offset + along_at(&taxi.schedule, end);
        let speed = ((along - motion.along()) / TICK.as_secs_f64()).max(0.0);
        motion.taxi_at(along, speed);
        let finish = taxi.schedule.last().map_or(0, |s| s.0);
        let done =
            taxi.departed + 1 >= taxi.route.steps.len() && end >= finish.max(taxi.route.arrival());
        if !done {
            return None;
        }
        let (airport, target, delay) = (
            taxi.airport,
            taxi.target,
            end - taxi.started - taxi.unimpeded,
        );
        self.stats
            .taxied(airport, SimDuration::from_millis(delay.max(0)));
        match target {
            Target::Hold => {
                motion.queue_now();
                self.taxis[index] = None;
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
                }
                None
            }
        }
    }

    /// Leaves the nodes due by `end` after checking that the way ahead is
    /// still free; `false` (and blocked) if it is not.
    pub(super) fn advance_steps(&mut self, index: usize, end: i64, now: SimTime) -> bool {
        let owner = Self::owner(index);
        loop {
            let Some(taxi) = self.taxis[index].as_ref() else {
                return false;
            };
            let at = taxi.departed;
            let (Some(&step), Some(&next)) =
                (taxi.route.steps.get(at), taxi.route.steps.get(at + 1))
            else {
                return true;
            };
            let airport = taxi.airport;
            let Some(airside) = self.ground.airside(airport) else {
                return true;
            };
            let runway_at =
                |node: Id<Node>| airside.runway_of.get(node.as_usize()).copied().flatten();
            // Onto a runway other than the one it is on: a crossing.
            let entering = runway_at(next.node).filter(|r| runway_at(step.node) != Some(*r));
            if end < step.leave {
                // Holding short: ask for the crossing while waiting.
                if let Some(runway) = entering.filter(|_| ms(now) >= step.arrive) {
                    self.hold_short(index, RunwayKey { airport, runway }, now);
                }
                return true;
            }
            let last = at + 2 == taxi.route.steps.len();
            let stay = if last && taxi.route.open {
                QUEUE_ESTIMATE
            } else {
                0
            };
            let edge_ok = next.via.is_none_or(|(edge, forward)| {
                airside
                    .table
                    .edge_free_after(edge, step.leave, next.arrive, forward, owner)
                    .is_none()
            });
            let node_ok = !airside.table.node_taken(
                next.node,
                next.arrive - NODE_BUFFER,
                next.leave.max(next.arrive + stay) + NODE_BUFFER,
                owner,
            );
            let runway = runway_at(next.node);
            let crossing_ok = entering.is_none_or(|runway| {
                self.control.may_cross(
                    RunwayKey { airport, runway },
                    now,
                    time_of(next.leave + NODE_BUFFER),
                )
            });
            if !(edge_ok && node_ok && crossing_ok) {
                if let Some(runway) = entering.filter(|_| !crossing_ok) {
                    self.hold_short(index, RunwayKey { airport, runway }, now);
                }
                self.block(index, step.node, ms(now));
                return false;
            }
            // On the runway (crossing or vacating): it stays occupied.
            if let Some(runway) = runway {
                self.control.crossing(
                    RunwayKey { airport, runway },
                    time_of(next.leave + NODE_BUFFER),
                );
            }
            if let Some(taxi) = self.taxis[index].as_mut() {
                if entering.is_some() {
                    taxi.crossing = None;
                }
                taxi.departed += 1;
            }
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

    /// Stops at `node`: frees the rest of the plan and holds the node.
    pub(super) fn block(&mut self, index: usize, node: Id<Node>, now: i64) {
        let owner = Self::owner(index);
        let Some(taxi) = self.taxis[index].as_mut() else {
            return;
        };
        taxi.blocked = Some((now, now + RETRY));
        let airport = taxi.airport;
        if let Some(airside) = self.ground.airside_mut(airport) {
            airside.table.release(owner);
            airside.table.hold(node, now, now + RETRY, owner);
        }
    }

    /// A blocked tail plans again from where it stands; after
    /// `ESCAPE_AFTER` it goes regardless (D75).
    pub(super) fn replan(&mut self, scenario: &Scenario, index: usize, now: SimTime, since: i64) {
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        let at = taxi.departed;
        let Some(here) = taxi.route.steps.get(at).map(|s| s.node) else {
            return;
        };
        let to = taxi.route.steps.last().map_or(here, |s| s.node);
        let plan = Plan {
            start_wait: i64::MAX,
            shared: true,
            airport: taxi.airport,
            from: here,
            to,
            start: ms(now),
            stay: if taxi.route.open {
                if taxi.target == Target::Hold {
                    QUEUE_ESTIMATE
                } else {
                    SPOT_ESTIMATE
                }
            } else {
                0
            },
            open: taxi.route.open,
        };
        let escape = ms(now) - since >= ESCAPE_AFTER;
        let owner = Self::owner(index);
        if let Some(airside) = self.ground.airside_mut(plan.airport) {
            airside.table.release(owner);
        }
        let Some(route) = self.plan_taxi(scenario, index, &plan, escape) else {
            if let Some(airside) = self.ground.airside_mut(plan.airport) {
                airside.table.hold(here, ms(now), ms(now) + RETRY, owner);
            }
            if let Some(taxi) = self.taxis[index].as_mut() {
                taxi.blocked = Some((since, ms(now) + RETRY));
            }
            return;
        };
        if escape {
            self.stats.ground_conflict(plan.airport);
        }
        self.continue_from(scenario, index, &plan, route, at);
    }

    /// Replaces the rest of a taxi from step `at` with `route`.
    pub(super) fn continue_from(
        &mut self,
        scenario: &Scenario,
        index: usize,
        plan: &Plan,
        route: TimedRoute,
        at: usize,
    ) {
        let Some(layout) = scenario.ground().airport(plan.airport) else {
            return;
        };
        let Some(taxi) = self.taxis[index].as_ref() else {
            return;
        };
        let (target, started, unimpeded) = (taxi.target, taxi.started, taxi.unimpeded);
        let mut base = taxi.base.clone();
        let travelled = taxi.route.points_until(layout, at);
        base.extend(travelled.iter().skip(usize::from(!base.is_empty())));
        let offset = taxi.offset
            + taxi
                .route
                .steps
                .get(at)
                .map_or(0.0, |s| along_at(&taxi.schedule, s.arrive));
        let kind_out = target == Target::Hold;
        let queue = if kind_out {
            self.paths[index]
                .as_ref()
                .map_or_else(Vec::new, |p| p.queue_points().to_vec())
        } else {
            Vec::new()
        };
        let Some(polyline) = self.start_taxi(
            scenario,
            index,
            plan,
            route,
            Start {
                target,
                base,
                offset,
                extension: queue,
            },
        ) else {
            return;
        };
        if let Some(taxi) = self.taxis[index].as_mut() {
            taxi.started = started;
            taxi.unimpeded = unimpeded;
        }
        if let Some(path) = self.paths[index].as_mut() {
            if kind_out {
                let length = path.set_taxi_out(polyline);
                self.profiles[index].set_taxi_out_length(length);
            } else {
                let heading = path.stand_in_heading().unwrap_or_default();
                let length = path.set_taxi_in(polyline, heading);
                self.profiles[index].set_taxi_in_length(length);
            }
        }
    }
}
