//! Deterministic, portable random numbers.
//!
//! Every subsystem draws from its own `ChaCha8` stream derived from one root
//! seed. A subsystem's sequence therefore depends only on the seed and its own
//! draws: adding a subsystem, or changing how often another one draws, never
//! perturbs it.

use anyhow::Result;
use rand_chacha::ChaCha8Rng;
use rand_core::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::CoreError;

/// Identifies one subsystem's independent random stream.
///
/// The discriminant is the `ChaCha` stream number, so existing variants must
/// never be renumbered; new subsystems get new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u64)]
pub enum StreamId {
    /// Schedule generation.
    Scheduling = 1,
    /// Day-to-day operations: delays, turnaround variation.
    Operations = 2,
}

/// The root seed of a simulation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RngSeed(u64);

impl RngSeed {
    /// Wraps a seed value.
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The raw seed value.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// A fresh generator for `stream`, positioned at its start.
    pub fn stream(self, stream: StreamId) -> SimRng {
        let mut inner = ChaCha8Rng::seed_from_u64(self.0);
        inner.set_stream(stream as u64);
        SimRng(inner)
    }
}

/// A deterministic random generator for one subsystem.
///
/// `ChaCha8` output is identical on every platform, including wasm.
#[derive(Debug, Clone, PartialEq)]
pub struct SimRng(ChaCha8Rng);

impl SimRng {
    /// A uniformly random `u32`.
    pub fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }

    /// A uniformly random `u64`.
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// A uniformly random float in `[0, 1)` with 53 bits of precision.
    #[allow(clippy::cast_precision_loss)]
    pub fn next_f64(&mut self) -> f64 {
        // The top 53 bits fit the f64 mantissa exactly, so the cast is
        // lossless.
        (self.next_u64() >> 11) as f64 * (1.0 / (1_u64 << 53) as f64)
    }

    /// A uniformly random integer in `0..bound`, without modulo bias (Lemire's
    /// method).
    #[allow(clippy::cast_possible_truncation)]
    pub fn below(&mut self, bound: u32) -> Result<u32> {
        if bound == 0 {
            return Err(CoreError::EmptyRange.into());
        }
        let mut product = u64::from(self.next_u32()) * u64::from(bound);
        // SAFETY: truncation to the low 32 bits is the point of Lemire's
        // method.
        if (product as u32) < bound {
            let threshold = bound.wrapping_neg() % bound;
            while (product as u32) < threshold {
                product = u64::from(self.next_u32()) * u64::from(bound);
            }
        }
        // SAFETY: `product < 2^32 * bound`, so the high half is below `bound`.
        Ok((product >> 32) as u32)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    const GOLDEN: [u64; 3] = [
        3_826_576_519_468_827_491,
        7_008_964_838_555_306_678,
        2_037_012_518_447_089_636,
    ];

    fn draws(rng: &mut SimRng, n: usize) -> Vec<u64> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(rng.next_u64());
        }
        out
    }

    #[test]
    fn test_rng_same_seed_same_sequence() {
        let mut a = RngSeed::new(42).stream(StreamId::Scheduling);
        let mut b = RngSeed::new(42).stream(StreamId::Scheduling);
        assert_eq!(draws(&mut a, 64), draws(&mut b, 64));
        assert_eq!(a, b);
    }

    #[test]
    fn test_rng_different_seeds_diverge() {
        let mut a = RngSeed::new(1).stream(StreamId::Scheduling);
        let mut b = RngSeed::new(2).stream(StreamId::Scheduling);
        assert_ne!(draws(&mut a, 8), draws(&mut b, 8));
    }

    #[test]
    fn test_rng_streams_are_distinct() {
        let seed = RngSeed::new(7);
        let mut scheduling = seed.stream(StreamId::Scheduling);
        let mut operations = seed.stream(StreamId::Operations);
        assert_ne!(draws(&mut scheduling, 8), draws(&mut operations, 8));
    }

    #[test]
    fn test_rng_stream_independent_of_other_streams_draws() {
        let seed = RngSeed::new(7);
        let mut busy = seed.stream(StreamId::Operations);
        let _ = draws(&mut busy, 1000);
        let mut fresh = seed.stream(StreamId::Scheduling);
        let mut reference = RngSeed::new(7).stream(StreamId::Scheduling);
        assert_eq!(draws(&mut fresh, 16), draws(&mut reference, 16));
    }

    #[test]
    fn test_rng_golden_values_are_stable() {
        // Pins the algorithm: if this changes, saved seeds no longer reproduce
        // runs.
        let mut rng = RngSeed::new(2026).stream(StreamId::Scheduling);
        assert_eq!(draws(&mut rng, 3), GOLDEN);
    }

    #[test]
    fn test_rng_below_rejects_zero_bound() -> Result<()> {
        let mut rng = RngSeed::new(0).stream(StreamId::Operations);
        let error = rng.below(0).err().context("zero bound must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::EmptyRange)
        );
        Ok(())
    }

    #[test]
    fn test_rng_seed_serde_round_trip() -> Result<()> {
        let seed = RngSeed::new(99);
        let json = serde_json::to_string(&seed).context("serialise")?;
        assert_eq!(json, "99");
        assert_eq!(
            serde_json::from_str::<RngSeed>(&json).context("deserialise")?,
            seed
        );
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_rng_next_f64_in_unit_interval(seed in any::<u64>()) {
            let mut rng = RngSeed::new(seed).stream(StreamId::Operations);
            for _ in 0..32 {
                let value = rng.next_f64();
                prop_assert!((0.0..1.0).contains(&value));
            }
        }

        #[test]
        fn test_rng_below_stays_below_bound(seed in any::<u64>(), bound in 1_u32..) {
            let mut rng = RngSeed::new(seed).stream(StreamId::Operations);
            for _ in 0..32 {
                let value = rng.below(bound).map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert!(value < bound);
            }
        }
    }
}
