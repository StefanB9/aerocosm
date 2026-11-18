//! Slot coordination (plan D54): movements booked per coordinated airport in
//! short windows of its local week, within the declared capacity.

use super::rotation::WEEK;
use crate::traffic::Capacity;

/// Which way a movement goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Way {
    Arrival,
    Departure,
}

/// Bookings at one airport, in windows of its local week: all movements,
/// and arrivals and departures apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotBook {
    capacity: Capacity,
    windows: Vec<u16>,
    arrivals: Vec<u16>,
    departures: Vec<u16>,
}

impl SlotBook {
    /// An empty week.
    pub(crate) fn new(capacity: Capacity) -> Self {
        let count = usize::try_from(WEEK / u32::from(capacity.window_minutes.max(1))).unwrap_or(0);
        Self {
            capacity,
            windows: vec![0; count],
            arrivals: vec![0; count],
            departures: vec![0; count],
        }
    }

    fn window(&self, minute: u32) -> usize {
        let index = (minute % WEEK) / u32::from(self.capacity.window_minutes.max(1));
        usize::try_from(index).unwrap_or(0) % self.windows.len().max(1)
    }

    /// Whether `booked` plus `extra` keeps every rolling hour of windows
    /// that contains a window of `extra` within `limit`.
    fn hours_fit(&self, booked: &[u16], extra: &[(usize, u16)], limit: u16) -> bool {
        let count = booked.len();
        let per_hour = usize::from(self.capacity.windows_per_hour());
        let with = |index: usize| {
            u32::from(booked[index])
                + extra
                    .iter()
                    .filter(|(w, _)| *w == index)
                    .map(|(_, n)| u32::from(*n))
                    .sum::<u32>()
        };
        extra.iter().all(|&(window, _)| {
            (0..per_hour).all(|back| {
                let start = (window + count - back) % count;
                let total: u32 = (0..per_hour).map(|step| with((start + step) % count)).sum();
                total <= u32::from(limit)
            })
        })
    }

    /// Movements of `movements` per window, counted.
    fn per_window(&self, movements: &[(u32, Way)], way: Option<Way>) -> Vec<(usize, u16)> {
        let mut counts: Vec<(usize, u16)> = Vec::with_capacity(movements.len());
        for &(minute, moving) in movements {
            if way.is_some_and(|w| w != moving) {
                continue;
            }
            let window = self.window(minute);
            match counts.iter_mut().find(|(w, _)| *w == window) {
                Some((_, n)) => *n = n.saturating_add(1),
                None => counts.push((window, 1)),
            }
        }
        counts
    }

    /// Whether `movements` at local week minutes (any multiple of a week is
    /// wrapped) fit together: each window and every rolling hour of windows
    /// containing them stays within the capacity, arrivals and departures
    /// within their own hourly limits where there are any.
    pub(crate) fn fits(&self, movements: &[(u32, Way)]) -> bool {
        if self.windows.is_empty() {
            return true;
        }
        let all = self.per_window(movements, None);
        let windows_fit = all
            .iter()
            .all(|&(window, n)| self.windows[window].saturating_add(n) <= self.capacity.per_window);
        let way_fits = |way: Way, booked: &[u16], limit: Option<u16>| {
            limit.is_none_or(|limit| {
                self.hours_fit(booked, &self.per_window(movements, Some(way)), limit)
            })
        };
        windows_fit
            && self.hours_fit(&self.windows, &all, self.capacity.per_hour)
            && way_fits(
                Way::Arrival,
                &self.arrivals,
                self.capacity.arrivals_per_hour,
            )
            && way_fits(
                Way::Departure,
                &self.departures,
                self.capacity.departures_per_hour,
            )
    }

    /// Records `movements` at local week minutes.
    pub(crate) fn book(&mut self, movements: &[(u32, Way)]) {
        for &(minute, way) in movements {
            let window = self.window(minute);
            let by_way = match way {
                Way::Arrival => &mut self.arrivals,
                Way::Departure => &mut self.departures,
            };
            for list in [&mut self.windows, by_way] {
                if let Some(count) = list.get_mut(window) {
                    *count = count.saturating_add(1);
                }
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
            arrivals_per_hour: None,
            departures_per_hour: None,
        }
    }

    /// Movements at `minutes`, all of one `way`.
    fn all(way: Way, minutes: &[u32]) -> Vec<(u32, Way)> {
        minutes.iter().map(|&m| (m, way)).collect()
    }

    #[test]
    fn test_arrivals_and_departures_have_their_own_limits() {
        let mut book = SlotBook::new(Capacity {
            arrivals_per_hour: Some(2),
            departures_per_hour: Some(3),
            ..capacity(10, 10, 10)
        });
        book.book(&all(Way::Arrival, &[600, 620]));
        assert!(
            !book.fits(&all(Way::Arrival, &[640])),
            "a third arrival in the hour"
        );
        assert!(book.fits(&all(Way::Departure, &[640])));
        assert!(
            book.fits(&all(Way::Arrival, &[680])),
            "10:20-11:20 holds one"
        );
    }

    #[test]
    fn test_window_limit() {
        let mut book = SlotBook::new(capacity(100, 10, 2));
        assert!(book.fits(&all(Way::Departure, &[600])));
        book.book(&all(Way::Departure, &[600, 605]));
        assert!(
            !book.fits(&all(Way::Departure, &[609])),
            "the 10:00-10:10 window is full"
        );
        assert!(book.fits(&all(Way::Departure, &[610])));
        assert!(
            !book.fits(&all(Way::Departure, &[610, 611, 612])),
            "several movements of one trip count together"
        );
    }

    #[test]
    fn test_rolling_hour_limit() {
        let mut book = SlotBook::new(capacity(3, 10, 3));
        book.book(&all(Way::Departure, &[600, 620, 640]));
        assert!(
            !book.fits(&all(Way::Departure, &[655])),
            "four in 10:00-11:00"
        );
        assert!(
            book.fits(&all(Way::Departure, &[660])),
            "10:10-11:10 holds only two"
        );
        assert!(
            !book.fits(&all(Way::Departure, &[595])),
            "09:50-10:50 would hold four"
        );
    }

    #[test]
    fn test_week_wraps() {
        let mut book = SlotBook::new(capacity(2, 30, 2));
        book.book(&all(Way::Departure, &[WEEK - 10, WEEK - 5]));
        assert!(
            !book.fits(&all(Way::Departure, &[WEEK + 5])),
            "Monday 00:05 is in the hour after Sunday 23:50"
        );
        assert!(!book.fits(&all(Way::Departure, &[5])));
        assert!(book.fits(&all(Way::Departure, &[50])));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_arrivals_and_departures_stay_within_their_limits(
            requests in prop::collection::vec(prop::collection::vec((0_u32..WEEK * 2, any::<bool>()), 1..4), 1..200),
            arrivals in 1_u16..20,
            departures in 1_u16..20,
        ) {
            let capacity = Capacity {
                arrivals_per_hour: Some(arrivals),
                departures_per_hour: Some(departures),
                ..capacity(40, 10, 10)
            };
            let mut book = SlotBook::new(capacity);
            let mut booked: Vec<(u32, Way)> = Vec::new();
            for request in requests {
                let request: Vec<(u32, Way)> = request
                    .into_iter()
                    .map(|(m, arrival)| (m, if arrival { Way::Arrival } else { Way::Departure }))
                    .collect();
                if book.fits(&request) {
                    book.book(&request);
                    booked.extend(request.iter().map(|&(m, way)| (m % WEEK, way)));
                }
            }
            for start in (0..WEEK).step_by(10) {
                let in_hour = |way: Way| {
                    booked
                        .iter()
                        .filter(|(m, w)| *w == way && (*m + WEEK - start) % WEEK < 60)
                        .count()
                };
                prop_assert!(in_hour(Way::Arrival) <= usize::from(arrivals));
                prop_assert!(in_hour(Way::Departure) <= usize::from(departures));
            }
        }

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
                let request = all(Way::Departure, &request);
                if book.fits(&request) {
                    book.book(&request);
                    minutes.extend(request.iter().map(|(m, _)| m % WEEK));
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
