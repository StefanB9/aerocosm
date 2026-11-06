//! Landing slots on one runway, kept apart by wake separation (plan M3 D57).

use super::separation::arrival_spacing;
use crate::{SimTime, WakeCategory};

/// One booked landing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slot {
    pub(crate) time: SimTime,
    pub(crate) wake: WakeCategory,
    /// The aircraft's approach speed, m/s: spacing behind a leader depends on
    /// how fast the follower flies the final.
    pub(crate) speed: f64,
}

/// A runway's landing slots in time order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Timeline {
    slots: Vec<Slot>,
}

impl Timeline {
    /// Books the earliest landing time at or after `eta` that keeps wake
    /// separation with both neighbours, and returns it.
    pub(crate) fn book(&mut self, eta: SimTime, wake: WakeCategory, speed: f64) -> SimTime {
        let after = |earlier: &Slot, at: SimTime| {
            earlier
                .time
                .checked_add(arrival_spacing(earlier.wake, wake, speed))
                .map_or(at, |free| free.max(at))
        };
        for index in 0..=self.slots.len() {
            let time = index
                .checked_sub(1)
                .and_then(|before| self.slots.get(before))
                .map_or(eta, |earlier| after(earlier, eta));
            let fits = self.slots.get(index).is_none_or(|later| {
                time.checked_add(arrival_spacing(wake, later.wake, later.speed))
                    .is_ok_and(|needed| needed <= later.time)
            });
            if fits {
                self.slots.insert(index, Slot { time, wake, speed });
                return time;
            }
        }
        eta
    }

    /// The first booked landing strictly after `time`.
    pub(crate) fn next_after(&self, time: SimTime) -> Option<SimTime> {
        self.slots
            .iter()
            .map(|slot| slot.time)
            .find(|slot| *slot > time)
    }

    /// Forgets landings before `time`.
    pub(crate) fn prune(&mut self, time: SimTime) {
        self.slots.retain(|slot| slot.time >= time);
    }

    /// Booked landings.
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;
    use crate::SimDuration;

    const WAKES: [WakeCategory; 4] = [
        WakeCategory::Light,
        WakeCategory::Medium,
        WakeCategory::Heavy,
        WakeCategory::Super,
    ];

    fn at(seconds: i32) -> Result<SimTime> {
        "2026-07-08T10:00:00Z"
            .parse::<SimTime>()?
            .checked_add(SimDuration::from_secs(seconds))
    }

    #[test]
    fn test_first_booking_lands_at_its_eta() -> Result<()> {
        let mut timeline = Timeline::default();
        assert_eq!(
            timeline.book(at(300)?, WakeCategory::Medium, 70.0),
            at(300)?
        );
        Ok(())
    }

    #[test]
    fn test_bookings_keep_wake_separation_and_fill_gaps() -> Result<()> {
        let mut timeline = Timeline::default();
        timeline.book(at(0)?, WakeCategory::Heavy, 75.0);
        // A medium wanting to land 30 s behind the heavy waits for 5 NM.
        let behind = timeline.book(at(30)?, WakeCategory::Medium, 70.0);
        let five_miles = arrival_spacing(WakeCategory::Heavy, WakeCategory::Medium, 70.0);
        assert_eq!(behind, at(0)?.checked_add(five_miles)?);
        // A later request with an early ETA fits in the gap before the heavy.
        let early = timeline.book(at(-600)?, WakeCategory::Medium, 70.0);
        assert_eq!(early, at(-600)?);
        // But not into a gap too small for it.
        let squeezed = timeline.book(at(-30)?, WakeCategory::Medium, 70.0);
        assert!(squeezed > behind, "{squeezed} after {behind}");
        assert_eq!(timeline.len(), 4);
        Ok(())
    }

    #[test]
    fn test_next_after_and_prune() -> Result<()> {
        let mut timeline = Timeline::default();
        timeline.book(at(100)?, WakeCategory::Medium, 70.0);
        timeline.book(at(400)?, WakeCategory::Medium, 70.0);
        assert_eq!(timeline.next_after(at(150)?), Some(at(400)?));
        assert_eq!(timeline.next_after(at(500)?), None);
        timeline.prune(at(200)?);
        assert_eq!(timeline.len(), 1);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_any_booking_order_stays_separated(
            requests in prop::collection::vec((0_i32..7_200, 0_usize..4, 60.0_f64..85.0), 1..60),
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let mut timeline = Timeline::default();
            for (eta, wake, speed) in requests {
                let eta = at(eta).map_err(fail)?;
                let slot = timeline.book(eta, WAKES[wake], speed);
                prop_assert!(slot >= eta);
            }
            for pair in timeline.slots.windows(2) {
                let gap = pair[1].time.duration_since(pair[0].time);
                prop_assert!(gap >= arrival_spacing(pair[0].wake, pair[1].wake, pair[1].speed));
            }
        }
    }
}
