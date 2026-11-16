//! Summaries of timing samples.

/// Median, 95th percentile, worst and mean of a set of samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Summary {
    pub(crate) median: f64,
    pub(crate) p95: f64,
    pub(crate) max: f64,
    pub(crate) mean: f64,
}

impl Summary {
    /// The summary of `samples` (sorted in place); `None` if there are none
    /// or any is not a number.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn of(samples: &mut [f64]) -> Option<Self> {
        if samples.is_empty() || samples.iter().any(|s| s.is_nan()) {
            return None;
        }
        samples.sort_by(f64::total_cmp);
        let len = samples.len();
        // Nearest rank: the smallest sample with at least that share at or
        // below it.
        let rank = |percent: usize| {
            let at = (percent * len).div_ceil(100);
            samples[at.clamp(1, len) - 1]
        };
        Some(Self {
            median: rank(50),
            p95: rank(95),
            max: samples[len - 1],
            mean: samples.iter().sum::<f64>() / len as f64,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_summary_of_known_samples() {
        let mut samples: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        let summary = Summary::of(&mut samples);
        assert_eq!(
            summary,
            Some(Summary {
                median: 50.0,
                p95: 95.0,
                max: 100.0,
                mean: 50.5
            })
        );
        assert_eq!(Summary::of(&mut []), None);
        assert_eq!(Summary::of(&mut [1.0, f64::NAN]), None);
        assert_eq!(
            Summary::of(&mut [4.0]).map(|s| (s.median, s.p95, s.max)),
            Some((4.0, 4.0, 4.0))
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The figures are ordered and lie among the samples.
        #[test]
        fn test_summary_is_ordered(mut samples in proptest::collection::vec(0.0_f64..1e4, 1..300)) {
            let low = samples.iter().copied().fold(f64::INFINITY, f64::min);
            let summary = Summary::of(&mut samples);
            prop_assert!(summary.is_some());
            if let Some(s) = summary {
                prop_assert!(low <= s.median && s.median <= s.p95 && s.p95 <= s.max);
                prop_assert!(low <= s.mean && s.mean <= s.max);
                prop_assert!(samples.contains(&s.median) && samples.contains(&s.p95));
            }
        }
    }
}
