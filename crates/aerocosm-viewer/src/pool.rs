//! Sprites as a pool (plan M9 PR 2): only aircraft that are shown get one.
//! An aircraft keeps its slot while it stays shown, so the render world sees
//! few changes from frame to frame; slots of aircraft no longer shown are
//! reused before the pool grows.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_keeps_slots_while_shown() {
        let mut pool = Assignment::default();
        assert_eq!(pool.assign([3, 7, 9]), 3);
        let (a, b, c) = (pool.slot_of(3), pool.slot_of(7), pool.slot_of(9));
        assert!(a.is_some() && b.is_some() && c.is_some());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(pool.assign([9, 3, 7]), 3, "the same aircraft need no new slot");
        assert_eq!(
            (pool.slot_of(3), pool.slot_of(7), pool.slot_of(9)),
            (a, b, c),
            "each keeps its slot"
        );
    }

    #[test]
    fn test_assign_reuses_freed_slots_before_growing() {
        let mut pool = Assignment::default();
        pool.assign([1, 2, 3]);
        let freed = pool.slot_of(2);
        assert_eq!(pool.assign([1, 3, 5]), 3, "5 takes 2's slot");
        assert_eq!(pool.slot_of(2), None);
        assert_eq!(pool.slot_of(5), freed);
        assert_eq!(pool.assign([1, 3, 5, 6]), 4, "grows by one");
        assert_eq!(pool.airframe_in(3), Some(6));
    }

    #[test]
    fn test_assign_frees_everything_not_shown() {
        let mut pool = Assignment::default();
        pool.assign([4, 8]);
        assert_eq!(pool.assign([]), 2, "the pool never shrinks");
        assert_eq!(pool.slot_of(4), None);
        assert_eq!(pool.airframe_in(0), None);
        assert_eq!(pool.airframe_in(1), None);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1000))]

        #[test]
        fn test_assign_is_one_to_one(
            frames in proptest::collection::vec(
                proptest::collection::btree_set(0_usize..64, 0..32), 1..8
            )
        ) {
            let mut pool = Assignment::default();
            for shown in frames {
                let size = pool.assign(shown.iter().copied());
                proptest::prop_assert!(size >= shown.len());
                let mut slots = std::collections::BTreeSet::new();
                for &airframe in &shown {
                    let slot = pool.slot_of(airframe);
                    proptest::prop_assert!(slot.is_some_and(|s| s < size));
                    proptest::prop_assert!(slots.insert(slot));
                    proptest::prop_assert_eq!(slot.and_then(|s| pool.airframe_in(s)), Some(airframe));
                }
                for slot in 0..size {
                    if let Some(airframe) = pool.airframe_in(slot) {
                        proptest::prop_assert!(shown.contains(&airframe));
                    }
                }
            }
        }
    }
}
