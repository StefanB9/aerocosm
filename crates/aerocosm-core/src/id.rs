//! Typed indices into the simulation's entity tables.

use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
};

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// Index of an entity of type `T` in its owning table.
///
/// The type parameter only tags the index: an `Id<Airport>` cannot be used
/// where an `Id<Runway>` is expected. `T` needs no trait bounds because all
/// impls are written by hand.
pub struct Id<T> {
    index: u32,
    // `fn() -> T` keeps `Id<T>` `Send + Sync` and covariant regardless of `T`.
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// Creates an id from a raw table index.
    pub const fn new(index: u32) -> Self {
        Self {
            index,
            marker: PhantomData,
        }
    }

    /// Creates an id from a `usize` table position, failing if it does not fit
    /// in `u32`.
    pub fn from_usize(index: usize) -> Result<Self> {
        let raw = u32::try_from(index).map_err(|_| CoreError::IdOverflow { index })?;
        Ok(Self::new(raw))
    }

    /// The raw table index.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The table index as `usize`, for slice access.
    pub const fn as_usize(self) -> usize {
        // SAFETY: lossless on every target with at least 32-bit pointers, which
        // is all targets aerocosm supports (64-bit native, wasm32).
        self.index as usize
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.index)
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.index)
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    struct Airport;
    struct Runway;

    #[test]
    fn test_id_index_round_trip() {
        let id: Id<Airport> = Id::new(42);
        assert_eq!(id.index(), 42);
        assert_eq!(id.as_usize(), 42);
    }

    #[test]
    fn test_id_equality_and_ordering_follow_index() {
        let a: Id<Airport> = Id::new(1);
        let b: Id<Airport> = Id::new(2);
        assert_eq!(a, Id::new(1));
        assert!(a < b);
        let set: HashSet<Id<Airport>> = [a, b, Id::new(1)].into_iter().collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_id_is_copy_without_bounds_on_target() {
        let a: Id<Runway> = Id::new(7);
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn test_id_debug_shows_index() {
        let id: Id<Airport> = Id::new(3);
        assert_eq!(format!("{id:?}"), "Id(3)");
    }

    #[test]
    fn test_id_from_usize_accepts_u32_range() -> Result<()> {
        let id: Id<Airport> = Id::from_usize(5).context("5 fits in u32")?;
        assert_eq!(id.index(), 5);
        Ok(())
    }

    #[test]
    fn test_id_from_usize_rejects_overflow() -> Result<()> {
        let too_big = usize::try_from(u64::from(u32::MAX) + 1).context("64-bit target")?;
        let error = Id::<Airport>::from_usize(too_big)
            .err()
            .context("overflow must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::IdOverflow { index: too_big })
        );
        Ok(())
    }

    #[test]
    fn test_id_serde_is_plain_number() -> Result<()> {
        let id: Id<Airport> = Id::new(9);
        let json = serde_json::to_string(&id).context("serialise")?;
        assert_eq!(json, "9");
        let back: Id<Airport> = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back, id);
        Ok(())
    }
}
