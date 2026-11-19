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
/// is cancelled (plan M8 D171; calibrated on the world runs).
pub(super) const RECOVERY_THRESHOLD: SimDuration = SimDuration::from_minutes(180);

/// When a tail is ready for its next leg.
#[derive(Debug, Clone, Copy)]
pub(super) enum Ready {
    /// It reached the gate then, and turns (quickly, if late).
    Arrived(SimTime),
    /// It stands ready at the gate now.
    Now(SimTime),
}

impl Ready {
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
        Ok(self
            .schedule
            .tail(Id::from_usize(index)?)
            .context("tail out of range")?
            .base())
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
        let (leg, week, scheduled, ready) =
            self.next_flown_leg(index, back.0, back.1, Ready::Now(now))?;
        self.tails[index] = self.leg_ops((leg, week), scheduled, ready, now)?;
        self.plan_parking(index, now);
        Ok(())
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
    ) -> Result<(usize, LocalDate, SimTime, SimTime)> {
        let base = self.base_of(index)?;
        let count = self.rotation(index)?.flights.len();
        let (mut leg, mut week) = next_leg(leg, count, week)?;
        // Every cancelled trip moves a later one up; a week's worth at most.
        for _ in 0..count {
            let rotation = self.rotation(index)?;
            let scheduled = rotation.scheduled(leg, week)?;
            let at = ready.at(scheduled, rotation.turnaround)?;
            let origin = self.flight(index, leg)?.origin();
            if origin != base || at.duration_since(scheduled) <= RECOVERY_THRESHOLD {
                return Ok((leg, week, scheduled, at));
            }
            let (back, back_week) = self.cancel_trip(index, leg, week)?;
            (leg, week) = next_leg(back, count, back_week)?;
        }
        let rotation = self.rotation(index)?;
        let scheduled = rotation.scheduled(leg, week)?;
        let at = ready.at(scheduled, rotation.turnaround)?;
        Ok((leg, week, scheduled, at))
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
    fn returning_tail(operations: &Operations) -> Result<(usize, usize)> {
        for index in 0..operations.tails.len() {
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

    #[test]
    fn test_a_departure_held_hours_at_its_base_cancels() -> Result<()> {
        let (_, operations) = setup(StartMode::Warm, 3)?;
        let index = tail_at_base(&operations)?;
        let scheduled = operations.tails[index].scheduled;
        let late = scheduled.checked_add(SimDuration::from_minutes(181))?;
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
                operations
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
        let (next, next_week, scheduled, _) =
            operations.next_flown_leg(index, leg, week, Ready::Arrived(arrived))?;
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
        let arrived = due.checked_add(SimDuration::from_minutes(60))?;
        let (next, _, scheduled, ready) =
            operations.next_flown_leg(index, leg, week, Ready::Arrived(arrived))?;
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
        let (next, ..) = operations.next_flown_leg(index, before, week, Ready::Arrived(arrived))?;
        assert_eq!(next, leg);
        assert_eq!(operations.statistics().network().cancellations(), 0);
        Ok(())
    }
}
