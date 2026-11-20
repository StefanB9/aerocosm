//! Spare aircraft (plan M8 D170): pools of ten tails or more keep spares at
//! their base, which fly no rotation of their own and take over late trips.

use std::collections::HashMap;

use super::Tail;

/// Pools smaller than this keep no spare.
const SMALLEST_POOL: usize = 10;
/// One spare per this many tails, rounded down; at least one from
/// [`SMALLEST_POOL`] tails.
const TAILS_PER_SPARE: usize = 20;

/// How many spares a pool of `tails` keeps: none under ten tails, then one
/// per twenty rounded down, at least one. About 1 % of the world's fleet.
pub(crate) fn spare_count(tails: usize) -> usize {
    if tails < SMALLEST_POOL {
        0
    } else {
        (tails / TAILS_PER_SPARE).max(1)
    }
}

/// The spares of `tails`: per pool (airline, base and type), in the order
/// the pools first appear, each a copy of its pool's tail.
pub(crate) fn spares(tails: &[Tail]) -> Vec<Tail> {
    let mut index = HashMap::with_capacity(tails.len());
    let mut pools: Vec<(Tail, usize)> = Vec::new();
    for tail in tails {
        let pool = *index
            .entry((tail.airline, tail.base, tail.aircraft))
            .or_insert_with(|| {
                pools.push((*tail, 0));
                pools.len() - 1
            });
        pools[pool].1 += 1;
    }
    pools
        .into_iter()
        .flat_map(|(tail, count)| std::iter::repeat_n(tail, spare_count(count)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Id;

    fn tail(airline: u32, base: u32, aircraft: u32) -> Tail {
        Tail {
            airline: Id::new(airline),
            aircraft: Id::new(aircraft),
            base: Id::new(base),
            utc_offset_minutes: 60,
        }
    }

    #[test]
    fn test_spare_count_follows_pool_size() {
        assert_eq!(spare_count(0), 0);
        assert_eq!(spare_count(9), 0);
        assert_eq!(spare_count(10), 1);
        assert_eq!(spare_count(39), 1);
        assert_eq!(spare_count(40), 2);
        assert_eq!(spare_count(65), 3);
    }

    #[test]
    fn test_spares_are_kept_per_pool_in_order() {
        let mut tails = vec![tail(0, 1, 2); 40];
        tails.extend(vec![tail(3, 1, 2); 5]);
        tails.extend(vec![tail(0, 4, 2); 12]);
        tails.insert(3, tail(0, 4, 2));
        let spares = spares(&tails);
        assert_eq!(
            spares,
            vec![tail(0, 1, 2), tail(0, 1, 2), tail(0, 4, 2)],
            "two for the 40, one for the 13, none for the 5"
        );
    }
}
