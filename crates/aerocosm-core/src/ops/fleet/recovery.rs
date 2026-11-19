//! Airline recovery (plan M8): late tails turn quickly (D168), and as the
//! last resort a round trip expected to leave its base hours late is
//! cancelled as a whole (D171).

use anyhow::{Context, Result};

use super::Operations;
use crate::{
    Id, SimDuration, SimTime,
    ops::tail::{next_leg, ready_at},
    time::LocalDate,
    world::Airport,
};

/// A round trip expected to leave its base later than this after schedule
/// is cancelled (plan M8 D171). Calibrated on 72-hour world runs: 3 hours
/// cancel 0.6 % of departures, 2 hours 0.9 %, 90 minutes 1.3 %, within the
/// 1-2.5 % airlines cancelled in 2019 (EUROCONTROL 1.7 %, BTS 1.9 %).
pub(super) const RECOVERY_THRESHOLD: SimDuration = SimDuration::from_minutes(90);

/// When a tail is ready for its next leg.
#[derive(Debug, Clone, Copy)]
pub(super) enum Ready {
    /// It reached the gate then, and turns (quickly, if late).
    Arrived(SimTime),
    /// It stands ready at the gate now.
    Now(SimTime),
}

impl Ready {
    pub(super) const fn time(self) -> SimTime {
        match self {
            Self::Arrived(time) | Self::Now(time) => time,
        }
    }

    fn at(self, scheduled: SimTime, turnaround: SimDuration) -> Result<SimTime> {
        match self {
            Self::Arrived(arrived) => ready_at(arrived, scheduled, turnaround),
            Self::Now(now) => Ok(now),
        }
    }
}

impl Operations {
    /// The base of tail `index`.
    pub(super) fn base_of(&self, index: usize) -> Result<Id<Airport>> {
        self.bases
            .get(index)
            .copied()
            .context("aircraft out of range")
    }

    /// Whether tail `index`, at off-block and held at the gate until
    /// `leaves`, cancels its round trip instead (plan M8 D171).
    pub(super) fn cancels_at_gate(&self, index: usize, leaves: SimTime) -> Result<bool> {
        let tail = &self.tails[index];
        Ok(leaves.duration_since(tail.scheduled) > RECOVERY_THRESHOLD
            && self.flight(index, tail.leg)?.origin() == self.base_of(index)?)
    }

    /// Cancels tail `index`'s round trip at the gate at `now`.
    /// Its landing slot is given back, and it stays parked, ready for its
    /// next trip.
    pub(super) fn cancel_at_gate(&mut self, index: usize, now: SimTime) -> Result<()> {
        let tail = self.tails[index];
        let back = self.cancel_trip(index, tail.leg, tail.week)?;
        self.drop_landing(index);
        self.paths[index] = None;
        self.gate_blocked[index] = None;
        self.tails[index] = self.next_ops(index, back, Ready::Now(now))?;
        self.plan_parking(index, now);
        Ok(())
    }

    /// Hands aircraft `index`'s late trip `(leg, week)`, scheduled at
    /// `scheduled`, to another aircraft of its pool parked at the base,
    /// ready in time and due to leave later (plan M8 D169): the one due
    /// latest, so the late aircraft gains the most time. The two exchange
    /// rotations; returns the trip the late aircraft takes instead, `None`
    /// without a partner.
    fn swap(
        &mut self,
        index: usize,
        trip: (usize, LocalDate),
        scheduled: SimTime,
        now: SimTime,
    ) -> Result<Option<(usize, LocalDate)>> {
        let base = self.base_of(index)?;
        let mut best: Option<(usize, SimTime)> = None;
        for &other in &self.pools[self.pool_of[index]] {
            let tail = &self.tails[other];
            if other == index
                || self.is_spare(other)
                || tail.motion.is_some()
                || tail.scheduled <= scheduled
                || best.is_some_and(|(_, due)| due >= tail.scheduled)
            {
                continue;
            }
            let ready = tail.scheduled.checked_add(tail.late)?;
            if ready.duration_since(scheduled) <= RECOVERY_THRESHOLD
                && self.flight(other, tail.leg)?.origin() == base
            {
                best = Some((other, tail.scheduled));
            }
        }
        let Some((other, _)) = best else {
            return Ok(None);
        };
        let partner = self.tails[other];
        let (late, early) = (self.rotation_of[index], self.rotation_of[other]);
        self.rotation_of[index] = early;
        self.rotation_of[other] = late;
        self.aircraft_of[late.context("a spare is never late")?] = other;
        self.aircraft_of[early.context("a spare is no partner")?] = index;
        let ready = partner.scheduled.checked_add(partner.late)?.max(now);
        self.tails[other] = self.leg_ops(trip, scheduled, ready, now)?;
        self.plan_parking(other, now);
        // Looked at again on the next tick, for its new boarding time.
        self.look_at[other] = self.time;
        self.stats.swapped(base);
        Ok(Some((partner.leg, partner.week)))
    }

    /// Counts the round trip whose outbound is `leg` of `week` as cancelled,
    /// both legs; returns its return leg and week.
    fn cancel_trip(
        &mut self,
        index: usize,
        leg: usize,
        week: LocalDate,
    ) -> Result<(usize, LocalDate)> {
        let rotation = self.rotation(index)?;
        let count = rotation.flights.len();
        let scheduled = rotation.scheduled(leg, week)?;
        let (back, back_week) = next_leg(leg, count, week)?;
        let back_scheduled = rotation.scheduled(back, back_week)?;
        let origin = self.flight(index, leg)?.origin();
        let far_end = self.flight(index, back)?.origin();
        self.stats.cancelled(origin, scheduled);
        self.stats.cancelled(far_end, back_scheduled);
        Ok((back, back_week))
    }

    /// The leg tail `index` flies after `leg` of the week `week`, having
    /// reached the gate at `arrived`: its index, week, scheduled departure,
    /// and when the aircraft is ready for it. A round trip that would leave
    /// the tail's base more than [`RECOVERY_THRESHOLD`] late is cancelled,
    /// both its legs; the tail stays for its next trip. Away from its base a
    /// tail always flies home.
    pub(super) fn next_flown_leg(
        &mut self,
        index: usize,
        leg: usize,
        week: LocalDate,
        ready: Ready,
    ) -> Result<Option<(usize, LocalDate, SimTime, SimTime)>> {
        let base = self.base_of(index)?;
        let count = self.rotation(index)?.flights.len();
        let (mut leg, mut week) = next_leg(leg, count, week)?;
        let mut swapped = false;
        // Every cancelled trip moves a later one up; a week's worth at most.
        for _ in 0..count {
            let rotation = self.rotation(index)?;
            let scheduled = rotation.scheduled(leg, week)?;
            let at = ready.at(scheduled, rotation.turnaround)?;
            let origin = self.flight(index, leg)?.origin();
            if origin != base || at.duration_since(scheduled) <= RECOVERY_THRESHOLD {
                return Ok(Some((leg, week, scheduled, at)));
            }
            // Another aircraft first (D169), once; then cancel (D171).
            if !swapped
                && let Some(trip) = self.swap(index, (leg, week), scheduled, ready.time())?
            {
                swapped = true;
                (leg, week) = trip;
                continue;
            }
            // Then a spare (D170): the aircraft becomes the spare.
            if self.take_spare(index, (leg, week), scheduled, ready.time())? {
                return Ok(None);
            }
            let (back, back_week) = self.cancel_trip(index, leg, week)?;
            (leg, week) = next_leg(back, self.rotation(index)?.flights.len(), back_week)?;
        }
        let rotation = self.rotation(index)?;
        let scheduled = rotation.scheduled(leg, week)?;
        let at = ready.at(scheduled, rotation.turnaround)?;
        Ok(Some((leg, week, scheduled, at)))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::{
        ops::{StartMode, fleet::tests::setup},
        schedule::ScheduledFlight,
    };

    /// A tail with at least two round trips and a leg `leg` of its first
    /// week that returns to its base, followed by a full round trip.
    /// Alone in its pool, so no swap can stand in for a cancellation.
    fn returning_tail(operations: &Operations) -> Result<(usize, usize)> {
        for index in 0..operations.tails.len() {
            if operations.pools[operations.pool_of[index]].len() > 1 {
                continue;
            }
            let base = operations
                .schedule
                .tail(Id::from_usize(index)?)
                .context("tail")?
                .base();
            let flights = operations.schedule.flights_of(Id::from_usize(index)?);
            if let Some(leg) = (0..flights.len().saturating_sub(3))
                .find(|leg| flights[leg + 1].origin() == base && flights[leg + 2].origin() != base)
            {
                return Ok((index, leg));
            }
        }
        anyhow::bail!("no tail flies two round trips")
    }

    /// A parked tail whose current leg leaves its base, with a round trip
    /// after it in the same week.
    fn tail_at_base(operations: &Operations) -> Result<usize> {
        (0..operations.tails.len())
            .find(|&index| {
                let tail = operations.tails[index];
                let flights = operations
                    .schedule
                    .flights_of(Id::from_usize(index).unwrap_or(Id::new(0)));
                tail.motion.is_none()
                    && tail.leg + 3 < flights.len()
                    && operations.base_of(index).ok() == Some(flights[tail.leg].origin())
            })
            .context("no tail parked at its base")
    }

    /// The leg before `leg` of `week` in a rotation of `count` legs.
    fn previous(leg: usize, week: LocalDate, count: usize) -> Result<(usize, LocalDate)> {
        if leg == 0 {
            Ok((count - 1, week.add_days(-7)?))
        } else {
            Ok((leg - 1, week))
        }
    }

    /// A late trip and a partner to swap with: aircraft `late` of a pool,
    /// a trip `(leg, week)` of its rotation from the base scheduled at
    /// `scheduled`, before which aircraft `partner` of the same pool, parked
    /// at the base, is ready and after which it is due to leave.
    struct SwapCase {
        late: usize,
        leg: usize,
        week: LocalDate,
        scheduled: SimTime,
        partner: usize,
    }

    fn swap_case(operations: &Operations) -> Result<SwapCase> {
        for (partner, tail) in operations.tails.iter().enumerate() {
            let base = operations.base_of(partner)?;
            if operations.is_spare(partner)
                || tail.motion.is_some()
                || operations.flight(partner, tail.leg)?.origin() != base
            {
                continue;
            }
            let ready = tail.scheduled.checked_add(tail.late)?;
            for &late in &operations.pools[operations.pool_of[partner]] {
                if late == partner || operations.is_spare(late) {
                    continue;
                }
                let rotation = operations.rotation(late)?;
                let week = operations.tails[late].week;
                for week in [week.add_days(-7)?, week, week.add_days(7)?] {
                    for (leg, flight) in rotation.flights.iter().enumerate() {
                        let scheduled = rotation.scheduled(leg, week)?;
                        if flight.origin() == base
                            && scheduled < tail.scheduled
                            && ready.duration_since(scheduled) <= RECOVERY_THRESHOLD
                        {
                            return Ok(SwapCase {
                                late,
                                leg,
                                week,
                                scheduled,
                                partner,
                            });
                        }
                    }
                }
            }
        }
        anyhow::bail!("no pool with a partner to swap with")
    }

    #[test]
    fn test_a_late_aircraft_swaps_with_a_ready_one_of_its_pool() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let case = swap_case(&operations)?;
        let (rotation, partner_rotation) = (
            operations.rotation_of[case.late].context("late rotation")?,
            operations.rotation_of[case.partner].context("partner rotation")?,
        );
        let partner_trip = (
            operations.tails[case.partner].leg,
            operations.tails[case.partner].week,
        );
        let count = operations.rotation(case.late)?.flights.len();
        let (before, before_week) = previous(case.leg, case.week, count)?;
        let arrived = case.scheduled.checked_add(SimDuration::from_minutes(150))?;
        let (leg, week, ..) = operations
            .next_flown_leg(case.late, before, before_week, Ready::Arrived(arrived))?
            .context("flies on")?;
        // The partner flies the late trip; the late aircraft its partner's.
        assert_eq!(operations.rotation_of[case.partner], Some(rotation));
        assert_eq!(operations.rotation_of[case.late], Some(partner_rotation));
        assert_eq!(operations.aircraft_of[rotation], case.partner);
        assert_eq!(operations.aircraft_of[partner_rotation], case.late);
        let partner = operations.tails[case.partner];
        assert_eq!(
            (partner.leg, partner.week, partner.scheduled),
            (case.leg, case.week, case.scheduled)
        );
        assert!(partner.motion.is_none());
        assert_eq!(operations.look_at[case.partner], operations.time);
        if operations.statistics().network().cancellations() == 0 {
            assert_eq!((leg, week), partner_trip);
        }
        let base = operations.base_of(case.late)?;
        assert_eq!(operations.statistics().airport(base).swaps(), 1);
        assert_eq!(operations.statistics().network().swaps(), 1);
        Ok(())
    }

    #[test]
    fn test_a_departure_held_hours_at_its_base_cancels() -> Result<()> {
        let (_, operations) = setup(StartMode::Warm, 3)?;
        let index = tail_at_base(&operations)?;
        let scheduled = operations.tails[index].scheduled;
        let late = scheduled.checked_add(SimDuration::from_minutes(91))?;
        let soon = scheduled.checked_add(SimDuration::from_minutes(60))?;
        assert!(operations.cancels_at_gate(index, late)?);
        assert!(!operations.cancels_at_gate(index, soon)?);
        Ok(())
    }

    #[test]
    fn test_a_departure_held_hours_away_from_its_base_flies() -> Result<()> {
        let (_, operations) = setup(StartMode::Warm, 3)?;
        let index = (0..operations.tails.len())
            .find(|&index| {
                let tail = operations.tails[index];
                !operations.is_spare(index)
                    && operations
                        .flight(index, tail.leg)
                        .map(ScheduledFlight::origin)
                        .ok()
                        != operations.base_of(index).ok()
            })
            .context("no tail away from its base")?;
        let scheduled = operations.tails[index].scheduled;
        let late = scheduled.checked_add(SimDuration::from_minutes(600))?;
        assert!(!operations.cancels_at_gate(index, late)?);
        Ok(())
    }

    #[test]
    fn test_a_trip_cancelled_at_the_gate_waits_for_the_next() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let index = tail_at_base(&operations)?;
        let tail = operations.tails[index];
        let now = tail.scheduled.checked_add(SimDuration::from_minutes(10))?;
        operations.cancel_at_gate(index, now)?;
        let after = operations.tails[index];
        assert_eq!((after.leg, after.week), (tail.leg + 2, tail.week));
        assert!(after.motion.is_none());
        assert!(operations.paths[index].is_none());
        // Ready at once: it never left the gate.
        assert_eq!(after.off_block.max(now), after.off_block);
        assert_eq!(operations.statistics().network().cancellations(), 2);
        Ok(())
    }

    #[test]
    fn test_a_trip_hours_late_from_its_base_is_cancelled() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let (index, leg) = returning_tail(&operations)?;
        let week = operations.tails[index].week;
        let due = operations.rotation(index)?.scheduled(leg + 1, week)?;
        let arrived = due.checked_add(SimDuration::from_minutes(240))?;
        let (next, next_week, scheduled, _) = operations
            .next_flown_leg(index, leg, week, Ready::Arrived(arrived))?
            .context("flies on")?;
        // Both legs of the late round trip go; the tail waits for the next.
        assert_eq!((next, next_week), (leg + 3, week));
        assert_eq!(
            scheduled,
            operations.rotation(index)?.scheduled(leg + 3, week)?
        );
        let network = operations.statistics().network();
        assert_eq!(network.cancellations(), 2);
        Ok(())
    }

    #[test]
    fn test_a_trip_less_late_than_the_threshold_flies() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let (index, leg) = returning_tail(&operations)?;
        let week = operations.tails[index].week;
        let due = operations.rotation(index)?.scheduled(leg + 1, week)?;
        // Ten minutes late, quick turns of at most 72 minutes keep it under
        // the threshold.
        let arrived = due.checked_add(SimDuration::from_minutes(10))?;
        let (next, _, scheduled, ready) = operations
            .next_flown_leg(index, leg, week, Ready::Arrived(arrived))?
            .context("flies on")?;
        assert_eq!(next, leg + 1);
        assert_eq!(scheduled, due);
        // Late, so a quick turn: never more than the minimum after arriving.
        let turnaround = operations.rotation(index)?.turnaround;
        assert!(ready <= arrived.checked_add(turnaround)?);
        assert_eq!(operations.statistics().network().cancellations(), 0);
        Ok(())
    }

    #[test]
    fn test_a_late_return_from_the_far_end_is_never_cancelled() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let (index, leg) = returning_tail(&operations)?;
        let week = operations.tails[index].week;
        // Leg `leg` flies from the far end back to the base. Arriving at the
        // far end a day late, the tail still flies home.
        let before = leg.checked_sub(1).context("a leg before")?;
        let due = operations.rotation(index)?.scheduled(leg, week)?;
        let arrived = due.checked_add(SimDuration::from_minutes(24 * 60))?;
        let (next, ..) = operations
            .next_flown_leg(index, before, week, Ready::Arrived(arrived))?
            .context("flies on")?;
        assert_eq!(next, leg);
        assert_eq!(operations.statistics().network().cancellations(), 0);
        Ok(())
    }
}
