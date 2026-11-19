//! Spare aircraft (plan M8 D170): they fly no rotation of their own and
//! wait in remote parking at their base until a late trip needs them.

use anyhow::{Context, Result};

use super::{Operations, TailOps, recovery::RECOVERY_THRESHOLD, remote::Parking};
use crate::{SimDuration, SimTime, scenario::Scenario, time::LocalDate};

impl TailOps {
    /// A spare's state: nothing due, ready from `available`. Its week is
    /// kept only to fill the field.
    pub(super) const fn idle(week: LocalDate, available: SimTime) -> Self {
        Self {
            leg: 0,
            week,
            scheduled: available,
            off_block: SimTime::MAX,
            boarding_from: SimTime::MAX,
            late: SimDuration::ZERO,
            drawn: SimDuration::ZERO,
            motion: None,
        }
    }
}

impl Operations {
    /// Whether aircraft `index` is a spare, flying no rotation.
    pub(super) fn is_spare(&self, index: usize) -> bool {
        self.rotation_of.get(index).is_some_and(Option::is_none)
    }

    /// Parks spare `index` at the start: remote at a mapped base, so it
    /// takes no stand while idle.
    pub(super) fn place_spare(
        &mut self,
        scenario: &Scenario,
        index: usize,
        week: LocalDate,
    ) -> TailOps {
        if scenario.ground().airport(self.bases[index]).is_some() {
            self.parking[index] = Parking::Remote(None);
        }
        TailOps::idle(week, self.time)
    }

    /// Hands aircraft `index`'s late trip `(leg, week)`, scheduled at
    /// `scheduled`, with the rest of its rotation, to a spare of its pool
    /// ready by then plus the recovery threshold, the earliest ready
    /// (plan M8 D170). The late aircraft, at the gate at `now`, becomes the
    /// pool's spare. Returns whether a spare took over.
    pub(super) fn take_spare(
        &mut self,
        index: usize,
        trip: (usize, LocalDate),
        scheduled: SimTime,
        now: SimTime,
    ) -> Result<bool> {
        let mut best: Option<(SimTime, usize)> = None;
        for &other in &self.pools[self.pool_of[index]] {
            let tail = &self.tails[other];
            if !self.is_spare(other) || tail.motion.is_some() {
                continue;
            }
            let ready = tail.scheduled.max(now);
            if ready.duration_since(scheduled) <= RECOVERY_THRESHOLD
                && best.is_none_or(|(earliest, _)| ready < earliest)
            {
                best = Some((ready, other));
            }
        }
        let Some((ready, spare)) = best else {
            return Ok(false);
        };
        let rotation = self.rotation_of[index].context("a spare is never late")?;
        self.rotation_of[spare] = Some(rotation);
        self.rotation_of[index] = None;
        self.aircraft_of[rotation] = spare;
        self.tails[spare] = self.leg_ops(trip, scheduled, ready, now)?;
        // Towed in from remote parking (or kept on its stand) for the trip.
        self.plan_parking(spare, now);
        self.look_at[spare] = self.time;
        self.stats.spared(self.base_of(index)?);
        Ok(true)
    }

    /// A spare on the ground: towed off its stand when due, then idle.
    pub(super) fn step_spare(&mut self, index: usize, now: SimTime) {
        if let Parking::TowOff(at) = self.parking[index]
            && now >= at
        {
            self.release_stand(index);
            self.parking[index] = Parking::Remote(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::ops::{StartMode, fleet::stands::tests::mapped};

    #[test]
    fn test_spares_wait_remote_at_their_base() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Cold, 3)?;
        let tails = operations.schedule.tails().len();
        let spares = operations.schedule.spares().to_vec();
        assert_ne!(spares.len(), 0);
        assert_eq!(operations.airframe_count(), tails + spares.len());
        let mut remote = 0;
        for (offset, spare) in spares.iter().enumerate() {
            let index = tails + offset;
            assert!(operations.is_spare(index));
            assert_eq!(operations.base_of(index)?, spare.base());
            assert!(operations.parked[index].is_none());
            let mapped = scenario.ground().airport(spare.base()).is_some();
            let parking = operations.parking[index];
            assert_eq!(parking == Parking::Remote(None), mapped, "{parking:?}");
            remote += usize::from(mapped);
        }
        assert!(remote > 0);
        for _ in 0..60 {
            operations.step(&scenario)?;
        }
        for index in tails..operations.airframe_count() {
            let tail = operations.tails.get(index).context("spare")?;
            assert!(tail.motion.is_none() && operations.is_spare(index));
        }
        assert!(
            operations
                .flights()
                .all(|flight| flight.airframe().as_usize() < tails)
        );
        Ok(())
    }

    /// An aircraft of a pool with a spare, parked on a stand at its base
    /// before its current leg, which leaves the base; and the spare.
    fn spare_case(operations: &Operations) -> Result<(usize, usize)> {
        let tails = operations.schedule.tails().len();
        for spare in tails..operations.airframe_count() {
            for &late in &operations.pools[operations.pool_of[spare]] {
                let tail = &operations.tails[late];
                if !operations.is_spare(late)
                    && tail.motion.is_none()
                    && operations.parked[late].is_some()
                    && operations.flight(late, tail.leg)?.origin() == operations.base_of(late)?
                {
                    return Ok((late, spare));
                }
            }
        }
        anyhow::bail!("no pool with a spare and an aircraft on a stand at its base")
    }

    /// No other aircraft of `late`'s pool is due later: none to swap with.
    /// Returns its state as it arrives from the leg before its current one.
    fn without_partners(operations: &mut Operations, late: usize) -> Result<TailOps> {
        for other in operations.pools[operations.pool_of[late]].clone() {
            if other != late && !operations.is_spare(other) {
                operations.tails[other].scheduled = SimTime::MIN;
            }
        }
        let tail = operations.tails[late];
        let count = operations.rotation(late)?.flights.len();
        let (leg, week) = if tail.leg == 0 {
            (count - 1, tail.week.add_days(-7)?)
        } else {
            (tail.leg - 1, tail.week)
        };
        Ok(TailOps { leg, week, ..tail })
    }

    #[test]
    fn test_a_late_trip_without_a_swap_is_taken_over_by_a_spare() -> Result<()> {
        let (_, mut operations) = mapped(StartMode::Cold, 3)?;
        let (late, spare) = spare_case(&operations)?;
        let arriving = without_partners(&mut operations, late)?;
        let tail = operations.tails[late];
        let rotation = operations.rotation_of[late].context("rotation")?;
        // 80 minutes late: its turn takes it past the threshold, while the
        // spare stands ready now.
        let arrived = tail.scheduled.checked_add(SimDuration::from_minutes(80))?;
        let next = operations.next_tail_leg(late, &arriving, arrived)?;
        // The spare flies the late trip and the rotation on from it…
        assert_eq!(operations.rotation_of[spare], Some(rotation));
        assert_eq!(operations.aircraft_of[rotation], spare);
        let taken = operations.tails[spare];
        assert_eq!(
            (taken.leg, taken.week, taken.scheduled),
            (tail.leg, tail.week, tail.scheduled)
        );
        assert!(taken.off_block >= arrived);
        assert_eq!(operations.look_at[spare], operations.time);
        // …and the late aircraft is the pool's spare now, towed off its
        // stand after deplaning.
        assert!(operations.is_spare(late));
        assert_eq!(
            (next.off_block, next.boarding_from),
            (SimTime::MAX, SimTime::MAX)
        );
        operations.tails[late] = next;
        operations.plan_parking(late, arrived);
        let Parking::TowOff(at) = operations.parking[late] else {
            anyhow::bail!("{:?}", operations.parking[late]);
        };
        operations.step_spare(late, at);
        assert_eq!(operations.parking[late], Parking::Remote(None));
        assert!(operations.parked[late].is_none());
        let base = operations.base_of(late)?;
        let stats = operations.statistics();
        assert_eq!(stats.airport(base).spares(), 1);
        assert_eq!(stats.network().cancellations(), 0);
        Ok(())
    }

    #[test]
    fn test_a_spare_too_late_itself_leaves_the_trip_cancelled() -> Result<()> {
        let (_, mut operations) = mapped(StartMode::Cold, 3)?;
        let (late, spare) = spare_case(&operations)?;
        let arriving = without_partners(&mut operations, late)?;
        let tail = operations.tails[late];
        // Arriving 150 minutes late, even a spare ready at once would leave
        // past the threshold.
        let arrived = tail.scheduled.checked_add(SimDuration::from_minutes(150))?;
        let next = operations.next_tail_leg(late, &arriving, arrived)?;
        assert!(operations.is_spare(spare) && !operations.is_spare(late));
        assert!(next.scheduled > tail.scheduled);
        let stats = operations.statistics().network();
        assert_eq!((stats.spares(), stats.cancellations() >= 2), (0, true));
        Ok(())
    }
}
