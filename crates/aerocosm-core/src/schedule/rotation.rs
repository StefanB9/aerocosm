//! Tail assignment: round trips grouped into aircraft rotations over a cyclic
//! week (D42).

/// Minutes in the template week.
pub(crate) const WEEK: u32 = 7 * 24 * 60;

/// A round trip from a base: the tail leaves at `start` and is back at `end`
/// (minutes since Monday 00:00, base-local; `end` may run past [`WEEK`] into
/// the next week) and then needs `turnaround` minutes before its next
/// departure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trip<K> {
    /// Trips only share tails within a pool (airline, base and aircraft type).
    pub(crate) pool: K,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) turnaround: u32,
}

/// The tail of every trip, and the pool of every tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assignment<K> {
    /// Tail index per trip, in input order.
    pub(crate) tails: Vec<usize>,
    /// Pool per tail.
    pub(crate) tail_pools: Vec<K>,
}

struct TailState {
    id: usize,
    first_start: u32,
    free_at: u32,
}

/// Assigns trips to as few tails as greedy best-fit allows: in start order,
/// each trip takes the tail of its pool that became free most recently but in
/// time, provided its return still leaves the tail ready for its first trip of
/// the following week; otherwise a new tail starts. Appending only ever adds
/// later trips, so checking the wrap on each append keeps every rotation
/// repeatable week after week.
pub(crate) fn assign<K: Copy + Ord>(trips: &[Trip<K>]) -> Assignment<K> {
    let mut order: Vec<usize> = (0..trips.len()).collect();
    order.sort_by_key(|index| (trips[*index].pool, trips[*index].start));
    let mut tails = vec![0; trips.len()];
    let mut tail_pools = Vec::with_capacity(trips.len() / 4 + 1);
    let mut open: Vec<TailState> = Vec::new();
    let mut current_pool = None;
    for index in order {
        let trip = trips[index];
        if current_pool != Some(trip.pool) {
            open.clear();
            current_pool = Some(trip.pool);
        }
        let ready = trip.end + trip.turnaround;
        let best = open
            .iter_mut()
            .filter(|tail| tail.free_at <= trip.start && ready <= tail.first_start + WEEK)
            .max_by_key(|tail| tail.free_at);
        let id = if let Some(tail) = best {
            tail.free_at = ready;
            tail.id
        } else {
            let id = tail_pools.len();
            tail_pools.push(trip.pool);
            open.push(TailState {
                id,
                first_start: trip.start,
                free_at: ready,
            });
            id
        };
        tails[index] = id;
    }
    Assignment { tails, tail_pools }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn trip(pool: u8, start: u32, duration: u32) -> Trip<u8> {
        Trip {
            pool,
            start,
            end: start + duration,
            turnaround: 30,
        }
    }

    /// Every trip of a tail, sorted by start.
    fn by_tail(trips: &[Trip<u8>], tails: &[usize]) -> Vec<Vec<Trip<u8>>> {
        let count = tails.iter().max().map_or(0, |m| m + 1);
        let mut grouped = vec![Vec::new(); count];
        for (trip, tail) in trips.iter().zip(tails) {
            grouped[*tail].push(*trip);
        }
        for list in &mut grouped {
            list.sort_by_key(|t| t.start);
        }
        grouped
    }

    #[test]
    fn test_sequential_trips_share_a_tail() {
        let trips = [trip(0, 360, 120), trip(0, 600, 120), trip(0, 900, 120)];
        let assignment = assign(&trips);
        assert_eq!(assignment.tails, vec![0, 0, 0]);
        assert_eq!(assignment.tail_pools, vec![0]);
    }

    #[test]
    fn test_overlapping_trips_need_two_tails() {
        let trips = [trip(0, 360, 300), trip(0, 400, 120)];
        let assignment = assign(&trips);
        assert_ne!(assignment.tails[0], assignment.tails[1]);
    }

    #[test]
    fn test_turnaround_is_respected() {
        // The second trip starts 20 minutes after the first ends; turnaround is
        // 30.
        let trips = [trip(0, 360, 120), trip(0, 500, 60)];
        let assignment = assign(&trips);
        assert_ne!(assignment.tails[0], assignment.tails[1]);
    }

    #[test]
    fn test_pools_never_share_tails() {
        let trips = [trip(0, 360, 60), trip(1, 600, 60)];
        let assignment = assign(&trips);
        assert_ne!(assignment.tails[0], assignment.tails[1]);
        assert_eq!(assignment.tail_pools.len(), 2);
    }

    #[test]
    fn test_week_wrap_is_respected() {
        // A Sunday-night trip returning on Monday 07:00 next week cannot be
        // followed by the same tail's Monday 06:00 departure.
        let trips = [trip(0, 360, 60), trip(0, WEEK - 60, 480)];
        let assignment = assign(&trips);
        assert_ne!(assignment.tails[0], assignment.tails[1]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_rotations_are_feasible(raw in prop::collection::vec((0_u8..3, 0_u32..WEEK, 30_u32..1200), 1..80)) {
            let trips: Vec<Trip<u8>> = raw.iter().map(|(pool, start, duration)| trip(*pool, *start, *duration)).collect();
            let assignment = assign(&trips);
            prop_assert_eq!(assignment.tails.len(), trips.len());
            for (tail, list) in by_tail(&trips, &assignment.tails).iter().enumerate() {
                for trip in list {
                    prop_assert_eq!(trip.pool, assignment.tail_pools[tail]);
                }
                for pair in list.windows(2) {
                    prop_assert!(pair[0].end + pair[0].turnaround <= pair[1].start, "overlap on tail {tail}");
                }
                if let (Some(first), Some(last)) = (list.first(), list.last()) {
                    prop_assert!(last.end + last.turnaround <= first.start + WEEK, "week wrap on tail {tail}");
                }
            }
            // Never more tails than trips, never fewer than the busiest moment needs.
            let peak = (0..WEEK).step_by(15).map(|t| trips.iter().filter(|x| x.start <= t && t < x.end).count()).max().unwrap_or(0);
            prop_assert!(assignment.tail_pools.len() <= trips.len());
            prop_assert!(assignment.tail_pools.len() >= peak.min(trips.len()));
        }
    }
}
