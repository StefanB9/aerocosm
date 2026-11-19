//! Airline recovery (plan M8): late tails turn quickly (D168), and as the
//! last resort a round trip expected to leave its base hours late is
//! cancelled as a whole (D171).

use anyhow::{Context, Result};

use super::Operations;
use crate::{
    Id, SimDuration, SimTime,
    ops::tail::{next_leg, ready_at},
    time::LocalDate,
};

/// A round trip expected to leave its base later than this after schedule
/// is cancelled (plan M8 D171; calibrated on the world runs).
pub(super) const RECOVERY_THRESHOLD: SimDuration = SimDuration::from_minutes(180);

impl Operations {
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
        arrived: SimTime,
    ) -> Result<(usize, LocalDate, SimTime, SimTime)> {
        let base = self
            .schedule
            .tail(Id::from_usize(index)?)
            .context("tail out of range")?
            .base();
        let count = self.rotation(index)?.flights.len();
        let (mut leg, mut week) = next_leg(leg, count, week)?;
        // Every cancelled trip moves a later one up; a week's worth at most.
        for _ in 0..count {
            let rotation = self.rotation(index)?;
            let scheduled = rotation.scheduled(leg, week)?;
            let ready = ready_at(arrived, scheduled, rotation.turnaround)?;
            let origin = self.flight(index, leg)?.origin();
            if origin != base || ready.duration_since(scheduled) <= RECOVERY_THRESHOLD {
                return Ok((leg, week, scheduled, ready));
            }
            let (back, back_week) = next_leg(leg, count, week)?;
            let back_scheduled = rotation.scheduled(back, back_week)?;
            let far_end = self.flight(index, back)?.origin();
            self.stats.cancelled(origin, scheduled);
            self.stats.cancelled(far_end, back_scheduled);
            (leg, week) = next_leg(back, count, back_week)?;
        }
        let rotation = self.rotation(index)?;
        let scheduled = rotation.scheduled(leg, week)?;
        let ready = ready_at(arrived, scheduled, rotation.turnaround)?;
        Ok((leg, week, scheduled, ready))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::ops::{StartMode, fleet::tests::setup};

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

    #[test]
    fn test_a_trip_hours_late_from_its_base_is_cancelled() -> Result<()> {
        let (_, mut operations) = setup(StartMode::Warm, 3)?;
        let (index, leg) = returning_tail(&operations)?;
        let week = operations.tails[index].week;
        let due = operations.rotation(index)?.scheduled(leg + 1, week)?;
        let arrived = due.checked_add(SimDuration::from_minutes(240))?;
        let (next, next_week, scheduled, _) =
            operations.next_flown_leg(index, leg, week, arrived)?;
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
        let (next, _, scheduled, ready) = operations.next_flown_leg(index, leg, week, arrived)?;
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
        let (next, ..) = operations.next_flown_leg(index, before, week, arrived)?;
        assert_eq!(next, leg);
        assert_eq!(operations.statistics().network().cancellations(), 0);
        Ok(())
    }
}
