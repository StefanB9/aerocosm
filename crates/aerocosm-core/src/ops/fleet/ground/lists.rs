//! Lists written during a tick and emptied before the next (plan M9 PR 3):
//! one per junction or chain of an airport, of which a tick writes few.
//! Emptying only those written keeps the survey from touching thousands of
//! empty lists at every busy airport on every tick.

/// One list per index, remembering which were written since the last clear.
#[derive(Debug, Clone)]
pub(super) struct TickLists<T> {
    lists: Vec<Vec<T>>,
    /// Indices of the lists not empty, each once.
    written: Vec<u32>,
}

/// Equal when the lists hold the same entries, whatever order they were
/// written in.
impl<T: PartialEq> PartialEq for TickLists<T> {
    fn eq(&self, other: &Self) -> bool {
        self.lists == other.lists
    }
}

impl<T> TickLists<T> {
    /// `len` empty lists.
    pub(super) fn new(len: usize) -> Self {
        Self {
            lists: std::iter::repeat_with(Vec::new).take(len).collect(),
            written: Vec::new(),
        }
    }

    /// Adds `value` to list `index`; false if there is no such list.
    pub(super) fn push(&mut self, index: usize, value: T) -> bool {
        let Some(list) = self.lists.get_mut(index) else {
            return false;
        };
        if list.is_empty() {
            // SAFETY: indices are one airport's junctions or chains, far
            // below u32::MAX, so every written list is recorded.
            if let Ok(index) = u32::try_from(index) {
                self.written.push(index);
            }
        }
        list.push(value);
        true
    }

    /// List `index`, if it exists.
    pub(super) fn get(&self, index: usize) -> Option<&Vec<T>> {
        self.lists.get(index)
    }

    /// Every list, by index.
    pub(super) fn iter(&self) -> std::slice::Iter<'_, Vec<T>> {
        self.lists.iter()
    }

    /// Empties the lists written since the last clear.
    pub(super) fn clear(&mut self) {
        for index in self.written.drain(..) {
            if let Some(list) = self.lists.get_mut(index as usize) {
                list.clear();
            }
        }
    }

    /// How many lists hold entries.
    #[cfg(test)]
    pub(super) fn written(&self) -> usize {
        self.written.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_lists_push_read_and_clear_only_what_was_written() {
        let mut lists = TickLists::new(4);
        assert!(lists.push(1, 'a'));
        assert!(lists.push(1, 'b'));
        assert!(lists.push(3, 'c'));
        assert!(!lists.push(4, 'd'), "out of range");
        assert_eq!(lists.get(1).map(Vec::as_slice), Some(&['a', 'b'][..]));
        assert_eq!(lists.get(0).map(Vec::len), Some(0));
        assert_eq!(lists.get(4), None);
        let lengths: Vec<usize> = lists.iter().map(Vec::len).collect();
        assert_eq!(lengths, [0, 2, 0, 1]);
        assert_eq!(lists.written(), 2, "each list counted once");
        lists.clear();
        assert!(lists.iter().all(Vec::is_empty));
        assert_eq!(lists.written(), 0);
        assert!(lists.push(0, 'e'));
        assert_eq!(lists.written(), 1);
        let mut other = TickLists::new(4);
        assert_ne!(lists, other);
        other.push(0, 'e');
        assert_eq!(lists, other, "equal by contents");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1000))]

        #[test]
        fn test_tick_lists_clear_leaves_every_list_empty(
            ticks in proptest::collection::vec(
                proptest::collection::vec((0_usize..40, 0_u32..1000), 0..60), 1..6
            )
        ) {
            let mut lists = TickLists::new(32);
            for writes in ticks {
                let mut expected = vec![Vec::new(); 32];
                for (index, value) in writes {
                    let accepted = lists.push(index, value);
                    proptest::prop_assert_eq!(accepted, index < 32);
                    if let Some(list) = expected.get_mut(index) {
                        list.push(value);
                    }
                }
                let actual: Vec<Vec<u32>> = lists.iter().cloned().collect();
                proptest::prop_assert_eq!(actual, expected);
                lists.clear();
                proptest::prop_assert!(lists.iter().all(Vec::is_empty));
            }
        }
    }
}
