//! A table with one optional entry per tail that knows which entries are
//! filled (plan M5 D136): a few thousand aircraft taxi out of a fleet of
//! tens of thousands, and ground control goes through them several times a
//! tick.

use std::ops::Index;

/// One optional `T` per tail, and the filled slots in ascending order.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ops::fleet) struct Table<T> {
    slots: Vec<Option<T>>,
    filled: Vec<u32>,
}

impl<T> Table<T> {
    /// An empty table for `len` tails.
    pub(in crate::ops::fleet) fn new(len: usize) -> Self {
        Self {
            slots: (0..len).map(|_| None).collect(),
            filled: Vec::new(),
        }
    }

    /// The entry of tail `index`, if it has one.
    pub(in crate::ops::fleet) fn get(&self, index: usize) -> Option<&T> {
        self.slots.get(index)?.as_ref()
    }

    /// The entry of tail `index`, to change in place.
    pub(in crate::ops::fleet) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.slots.get_mut(index)?.as_mut()
    }

    /// Gives tail `index` the entry `value`; tails beyond the table are
    /// ignored.
    pub(in crate::ops::fleet) fn set(&mut self, index: usize, value: T) {
        let (Some(slot), Ok(key)) = (self.slots.get_mut(index), u32::try_from(index)) else {
            return;
        };
        if slot.replace(value).is_none()
            && let Err(at) = self.filled.binary_search(&key)
        {
            self.filled.insert(at, key);
        }
    }

    /// Removes and returns the entry of tail `index`.
    pub(in crate::ops::fleet) fn clear(&mut self, index: usize) -> Option<T> {
        let value = self.slots.get_mut(index)?.take()?;
        if let Ok(at) = u32::try_from(index).map_or(Err(0), |key| self.filled.binary_search(&key)) {
            self.filled.remove(at);
        }
        Some(value)
    }

    /// The tails with an entry, ascending.
    pub(in crate::ops::fleet) fn filled(&self) -> &[u32] {
        &self.filled
    }
}

impl<T> Index<usize> for Table<T> {
    type Output = Option<T>;

    fn index(&self, index: usize) -> &Option<T> {
        self.slots.get(index).unwrap_or(&None)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_table_lists_filled_slots_in_order() {
        let mut table = Table::new(10);
        table.set(7, 'a');
        table.set(2, 'b');
        table.set(7, 'c');
        table.set(99, 'x');
        assert_eq!(table.filled(), [2, 7]);
        assert_eq!(table.get(7), Some(&'c'));
        assert_eq!(table[2], Some('b'));
        assert_eq!(table[3], None);
        assert_eq!(table[99], None);
        if let Some(entry) = table.get_mut(2) {
            *entry = 'd';
        }
        assert_eq!(table.clear(2), Some('d'));
        assert_eq!(table.clear(2), None);
        assert_eq!(table.filled(), [7]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The filled list is always exactly the slots that hold a value.
        #[test]
        fn test_filled_matches_the_slots(
            steps in proptest::collection::vec((0_usize..24, any::<bool>()), 0..200),
        ) {
            let mut table = Table::new(20);
            let mut model = [false; 20];
            for (index, set) in steps {
                if set {
                    table.set(index, index);
                } else {
                    table.clear(index);
                }
                if let Some(slot) = model.get_mut(index) {
                    *slot = set;
                }
                let expected: Vec<u32> = (0..20_u32).filter(|i| model[*i as usize]).collect();
                prop_assert_eq!(table.filled(), &expected[..]);
                for (i, filled) in model.iter().enumerate() {
                    prop_assert_eq!(table.get(i).is_some(), *filled);
                }
            }
        }
    }
}
