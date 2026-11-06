//! How an arrival absorbs its delay (plan M3 D57): speed control first,
//! holding for the rest.

/// Speed control stretches the remaining flight by at most this fraction…
const SPEED_SHARE: f64 = 0.10;
/// …and at most this many seconds.
pub(crate) const MAX_SPEED_DELAY: f64 = 300.0;
/// No aircraft is slowed below this fraction of its normal speed.
pub(crate) const MIN_SPEED_FACTOR: f64 = 0.85;

/// How an arrival flies its delay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ArrivalPlan {
    /// Fraction of the normal speed for the rest of cruise and descent.
    pub(crate) speed_factor: f64,
    /// Seconds to spend in the hold.
    pub(crate) hold_seconds: f64,
}

/// Splits `delay` seconds between speed control over the `remaining`
/// seconds of en-route flight and holding.
pub(crate) fn plan_arrival(delay: f64, remaining: f64) -> ArrivalPlan {
    let remaining = remaining.max(1.0);
    let budget = (remaining * SPEED_SHARE)
        .min(MAX_SPEED_DELAY)
        .min(remaining / MIN_SPEED_FACTOR - remaining);
    let by_speed = delay.max(0.0).min(budget);
    ArrivalPlan {
        speed_factor: remaining / (remaining + by_speed),
        hold_seconds: delay.max(0.0) - by_speed,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_small_delays_are_flown_off_by_slowing_down() {
        let plan = plan_arrival(120.0, 1_800.0);
        assert!(plan.hold_seconds.abs() < 1e-9);
        // 1,800 s stretched by 120 s.
        assert!((plan.speed_factor - 1_800.0 / 1_920.0).abs() < 1e-9);
    }

    #[test]
    fn test_large_delays_hold_after_the_speed_budget() {
        let plan = plan_arrival(900.0, 1_800.0);
        // 10 % of 1,800 s by speed, the rest in the hold.
        assert!((plan.speed_factor - 1_800.0 / 1_980.0).abs() < 1e-9);
        assert!((plan.hold_seconds - 720.0).abs() < 1e-9);
    }

    #[test]
    fn test_no_delay_needs_nothing() {
        let plan = plan_arrival(0.0, 1_200.0);
        assert!((plan.speed_factor - 1.0).abs() < 1e-12);
        assert!(plan.hold_seconds.abs() < 1e-12);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_plans_absorb_exactly_the_delay(delay in 0.0_f64..7_200.0, remaining in 60.0_f64..10_000.0) {
            let plan = plan_arrival(delay, remaining);
            prop_assert!((MIN_SPEED_FACTOR..=1.0).contains(&plan.speed_factor));
            prop_assert!(plan.hold_seconds >= 0.0);
            let by_speed = remaining / plan.speed_factor - remaining;
            prop_assert!((by_speed + plan.hold_seconds - delay).abs() < 1e-6);
            prop_assert!(by_speed <= MAX_SPEED_DELAY + 1e-6);
        }
    }
}
