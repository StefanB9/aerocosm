//! Spare aircraft (plan M8 D170): they fly no rotation of their own and
//! wait in remote parking at their base until a late trip needs them.

use anyhow::{Context, Result};

use super::{Operations, TailOps, recovery::RECOVERY_THRESHOLD, remote::Parking};
use crate::{
    SimDuration, SimTime,
    ops::{
        Recovery,
        tail::{next_leg, ready_at},
    },
    scenario::Scenario,
    time::LocalDate,
};

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
        let Some((ready, spare)) = self.ready_spare(index, scheduled, now) else {
            return Ok(false);
        };
        let rotation = self.rotation_of[index].context("a spare is never late")?;
        self.rotation_of[index] = None;
        self.took_over[index] = None;
        self.aircraft_of[rotation] = spare;
        self.assign_spare(spare, rotation, (trip, scheduled), ready, now)?;
        self.stats.spared(self.base_of(index)?);
        Ok(true)
    }

    /// The spare of `index`'s pool ready earliest, from `now`, and when, if
    /// that is within the recovery threshold of `scheduled`.
    fn ready_spare(
        &self,
        index: usize,
        scheduled: SimTime,
        now: SimTime,
    ) -> Option<(SimTime, usize)> {
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
        best
    }

    /// Spare `spare` flies `rotation` from `trip`, scheduled at the time
    /// given, ready at `ready`; decided at `now`.
    fn assign_spare(
        &mut self,
        spare: usize,
        rotation: usize,
        (trip, scheduled): ((usize, LocalDate), SimTime),
        ready: SimTime,
        now: SimTime,
    ) -> Result<()> {
        self.rotation_of[spare] = Some(rotation);
        self.tails[spare] = self.leg_ops(trip, scheduled, ready, now)?;
        // Towed in from remote parking (or kept on its stand) for the trip.
        self.plan_parking(spare, now);
        self.look_at[spare] = self.time;
        self.note_recovery(spare, Recovery::Spare, trip)
    }

    /// Aircraft `index` took off at `now` on leg `leg`. On its way back to
    /// its base, expected to be ready for its next trip more than the
    /// recovery threshold late, with no aircraft of its pool to swap with,
    /// it hands that trip and the rest of its rotation to a spare ready in
    /// time (plan M8 D170). It keeps flying this leg and becomes the spare
    /// when it reaches the gate.
    pub(super) fn dispatch_spare(&mut self, index: usize, leg: usize, now: SimTime) -> Result<()> {
        let base = self.base_of(index)?;
        let flight = *self.flight(index, leg)?;
        if self.relieved[index] || flight.destination() != base {
            return Ok(());
        }
        let tail = self.tails[index];
        let rotation = self.rotation(index)?;
        let trip = next_leg(leg, rotation.flights.len(), tail.week)?;
        let scheduled = rotation.scheduled(trip.0, trip.1)?;
        let landing = tail.off_block.checked_add(flight.block())?;
        let ready = ready_at(landing, scheduled, rotation.turnaround)?;
        if ready.duration_since(scheduled) <= RECOVERY_THRESHOLD
            || self.swap_partner(index, scheduled)?.is_some()
        {
            return Ok(());
        }
        let Some((ready, spare)) = self.ready_spare(index, scheduled, now) else {
            return Ok(());
        };
        let rotation = self.rotation_of[index].context("a spare is never late")?;
        self.assign_spare(spare, rotation, (trip, scheduled), ready, now)?;
        self.relieved[index] = true;
        self.stats.spared(base);
        Ok(())
    }

    /// Aircraft `index`, relieved by a spare on its way in, reaches the
    /// gate: it hands its rotation over and becomes the spare. Returns
    /// whether it was relieved.
    pub(super) fn hand_over(&mut self, index: usize) -> Result<bool> {
        if !std::mem::take(&mut self.relieved[index]) {
            return Ok(false);
        }
        let rotation = self.rotation_of[index].context("a spare is never relieved")?;
        self.rotation_of[index] = None;
        self.took_over[index] = None;
        // The spare, or whoever it has swapped the rotation with since.
        if let Some(&holder) = self.pools[self.pool_of[index]]
            .iter()
            .find(|&&other| self.rotation_of[other] == Some(rotation))
        {
            self.aircraft_of[rotation] = holder;
        }
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
mod tests;
