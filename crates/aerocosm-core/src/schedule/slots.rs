//! Slot coordination (plan D54): movements booked per coordinated airport in
//! short windows of its local week, within the declared capacity.

use super::rotation::WEEK;
use crate::traffic::Capacity;

/// Bookings at one coordinated airport, in windows of its local week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotBook {
    capacity: Capacity,
    windows: Vec<u16>,
}

impl SlotBook {
    /// An empty week.
    pub(crate) fn new(capacity: Capacity) -> Self {
        let count = WEEK / u32::from(capacity.window_minutes.max(1));
        Self {
            capacity,
            windows: vec![0; usize::try_from(count).unwrap_or(0)],
        }
    }

    fn window(&self, minute: u32) -> usize {
        let index = (minute % WEEK) / u32::from(self.capacity.window_minutes.max(1));
        usize::try_from(index).unwrap_or(0) % self.windows.len().max(1)
    }

    /// Whether movements at the local week `minutes` (any multiple of a
    /// week is wrapped) fit together: each window and every rolling hour of
    /// windows containing them stays within the capacity.
    pub(crate) fn fits(&self, minutes: &[u32]) -> bool {
        let count = self.windows.len();
        if count == 0 {
            return true;
        }
        let extra = |window: usize| {
            u16::try_from(
                minutes
                    .iter()
                    .filter(|m| self.window(**m) == window)
                    .count(),
            )
            .unwrap_or(u16::MAX)
        };
        let per_hour = usize::from(self.capacity.windows_per_hour());
        minutes.iter().all(|minute| {
            let window = self.window(*minute);
            if self.windows[window] + extra(window) > self.capacity.per_window {
                return false;
            }
            (0..per_hour).all(|back| {
                let start = (window + count - back) % count;
                let total: u32 = (0..per_hour)
                    .map(|step| {
                        let index = (start + step) % count;
                        u32::from(self.windows[index] + extra(index))
                    })
                    .sum();
                total <= u32::from(self.capacity.per_hour)
            })
        })
    }

    /// Records movements at the local week `minutes`.
    pub(crate) fn book(&mut self, minutes: &[u32]) {
        for minute in minutes {
            let window = self.window(*minute);
            if let Some(count) = self.windows.get_mut(window) {
                *count = count.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn capacity(per_hour: u16, window_minutes: u16, per_window: u16) -> Capacity {
        Capacity {
            per_hour,
            window_minutes,
            per_window,
        }
    }

    #[test]
    fn test_window_limit() {
        let mut book = SlotBook::new(capacity(100, 10, 2));
        assert!(book.fits(&[600]));
        book.book(&[600, 605]);
        assert!(!book.fits(&[609]), "the 10:00-10:10 window is full");
        assert!(book.fits(&[610]));
        assert!(
            !book.fits(&[610, 611, 612]),
            "several movements of one trip count together"
        );
    }

    #[test]
    fn test_rolling_hour_limit() {
        let mut book = SlotBook::new(capacity(3, 10, 3));
        book.book(&[600, 620, 640]);
        assert!(!book.fits(&[655]), "four in 10:00-11:00");
        assert!(book.fits(&[660]), "10:10-11:10 holds only two");
        assert!(!book.fits(&[595]), "09:50-10:50 would hold four");
    }

    #[test]
    fn test_week_wraps() {
        let mut book = SlotBook::new(capacity(2, 30, 2));
        book.book(&[WEEK - 10, WEEK - 5]);
        assert!(
            !book.fits(&[WEEK + 5]),
            "Monday 00:05 is in the hour after Sunday 23:50"
        );
        assert!(!book.fits(&[5]));
        assert!(book.fits(&[50]));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_bookings_never_exceed_capacity(
            requests in prop::collection::vec(prop::collection::vec(0_u32..WEEK * 2, 1..4), 1..200),
            per_hour in 1_u16..40,
            window in prop::sample::select(vec![5_u16, 10, 15, 20, 30]),
            per_window in 1_u16..12,
        ) {
            let capacity = capacity(per_hour, window, per_window.min(per_hour));
            let mut book = SlotBook::new(capacity);
            let mut minutes = Vec::new();
            for request in requests {
                if book.fits(&request) {
                    book.book(&request);
                    minutes.extend(request.iter().map(|m| m % WEEK));
                }
            }
            let window = u32::from(capacity.window_minutes);
            for start in (0..WEEK).step_by(usize::try_from(window).unwrap_or(1)) {
                let in_window = minutes.iter().filter(|m| (**m + WEEK - start) % WEEK < window).count();
                let in_hour = minutes.iter().filter(|m| (**m + WEEK - start) % WEEK < 60).count();
                prop_assert!(in_window <= usize::from(capacity.per_window));
                prop_assert!(in_hour <= usize::from(capacity.per_hour));
            }
        }
    }
}
