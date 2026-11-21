//! Sprites as a pool (plan M9 PR 2): only aircraft that are shown get one.
//! An aircraft keeps its slot while it stays shown, so the render world sees
//! few changes from frame to frame; slots of aircraft no longer shown are
//! reused before the pool grows.

/// Which aircraft holds which slot of the pool. Aircraft are plain indices
/// (an airframe's index); a slot is an index into the pool's sprites.
#[derive(Debug, Default)]
pub(crate) struct Assignment {
    /// The slot of each aircraft, by aircraft index.
    slot_of: Vec<Option<usize>>,
    /// The aircraft in each slot.
    airframe_in: Vec<Option<usize>>,
    /// Slots shown last frame, to free those not shown now.
    shown: Vec<usize>,
    /// Marks this frame's aircraft, by aircraft index; cleared after use.
    seen: Vec<bool>,
}

impl Assignment {
    /// Gives each of `shown` (distinct aircraft) a slot: the one it had if
    /// it was shown last frame, else a free one, else a new one. Frees the
    /// slots of aircraft no longer shown. Returns the pool's size, which
    /// never shrinks.
    pub(crate) fn assign(&mut self, shown: impl IntoIterator<Item = usize>) -> usize {
        let shown: Vec<usize> = shown.into_iter().collect();
        let needed = shown.iter().max().map_or(0, |&most| most + 1);
        if self.seen.len() < needed {
            self.seen.resize(needed, false);
            self.slot_of.resize(needed, None);
        }
        for &airframe in &shown {
            self.seen[airframe] = true;
        }
        // Free the slots of aircraft not shown this frame.
        for airframe in std::mem::take(&mut self.shown) {
            if !self.seen.get(airframe).copied().unwrap_or(false)
                && let Some(slot) = self.slot_of.get_mut(airframe).and_then(Option::take)
                && let Some(held) = self.airframe_in.get_mut(slot)
            {
                *held = None;
            }
        }
        let mut free = (0..self.airframe_in.len())
            .filter(|&slot| self.airframe_in[slot].is_none())
            .collect::<Vec<_>>()
            .into_iter();
        for &airframe in &shown {
            self.seen[airframe] = false;
            if self.slot_of[airframe].is_some() {
                continue;
            }
            let slot = free.next().unwrap_or_else(|| {
                self.airframe_in.push(None);
                self.airframe_in.len() - 1
            });
            self.airframe_in[slot] = Some(airframe);
            self.slot_of[airframe] = Some(slot);
        }
        self.shown = shown;
        self.airframe_in.len()
    }

    /// The slot `airframe` holds, if it is shown.
    pub(crate) fn slot_of(&self, airframe: usize) -> Option<usize> {
        self.slot_of.get(airframe).copied().flatten()
    }

    /// The aircraft holding `slot`, if any.
    pub(crate) fn airframe_in(&self, slot: usize) -> Option<usize> {
        self.airframe_in.get(slot).copied().flatten()
    }
}

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
        assert_eq!(
            pool.assign([9, 3, 7]),
            3,
            "the same aircraft need no new slot"
        );
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
