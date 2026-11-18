//! Departure delay by cause (plan M8 D167): the aircraft was late, the
//! airline's own delay, or flow control and the ground.

use crate::SimDuration;

/// Published shares of departure delay caused by late aircraft, for
/// validation only (plan M8 D167, D172): EUROCONTROL's CODA digest for
/// 2019, reactionary delay 5.7 of 13.1 minutes per departure; BTS for 2019,
/// late-arriving aircraft 39.7 % of delay minutes (its cause categories,
/// full year).
pub const LATE_AIRCRAFT_REFERENCES: [(&str, f64); 2] =
    [("CODA 2019", 5.7 / 13.1), ("BTS 2019", 0.397)];

/// Departure delay split by cause: totals, or averages per departure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DelayCauses {
    late_aircraft: i64,
    airline: i64,
    flow_and_ground: i64,
}

impl DelayCauses {
    /// Splits a departure's `delay`: its aircraft was ready `late` after
    /// schedule, and the airline's own delay drawn for the leg was `drawn`.
    pub(crate) fn split(delay: SimDuration, late: SimDuration, drawn: SimDuration) -> Self {
        let delay = delay.as_millis().max(0);
        let late_aircraft = late.as_millis().clamp(0, delay);
        let airline = (drawn.as_millis() - late_aircraft).clamp(0, delay - late_aircraft);
        Self {
            late_aircraft,
            airline,
            flow_and_ground: delay - late_aircraft - airline,
        }
    }

    /// Delay because the aircraft arrived too late to leave on time.
    pub const fn late_aircraft(self) -> SimDuration {
        SimDuration::from_millis(self.late_aircraft)
    }

    /// The airline's own delay beyond that (boarding, handling, technical).
    pub const fn airline(self) -> SimDuration {
        SimDuration::from_millis(self.airline)
    }

    /// The rest: flow control at the destination, waits for a way out.
    pub const fn flow_and_ground(self) -> SimDuration {
        SimDuration::from_millis(self.flow_and_ground)
    }

    /// The whole delay.
    pub const fn total(self) -> SimDuration {
        SimDuration::from_millis(self.late_aircraft + self.airline + self.flow_and_ground)
    }

    /// Share of the delay caused by late aircraft; 0 without delay.
    pub fn late_aircraft_share(self) -> f64 {
        let total = self.total().as_millis();
        if total == 0 {
            return 0.0;
        }
        // SAFETY: a run's delay stays far below 2^53 ms.
        #[allow(clippy::cast_precision_loss)]
        let share = self.late_aircraft as f64 / total as f64;
        share
    }

    pub(super) const fn add(&mut self, other: Self) {
        self.late_aircraft += other.late_aircraft;
        self.airline += other.airline;
        self.flow_and_ground += other.flow_and_ground;
    }

    /// Averages over `count` departures.
    pub(super) fn per(self, count: u32) -> Self {
        if count == 0 {
            return Self::default();
        }
        let count = i64::from(count);
        Self {
            late_aircraft: self.late_aircraft / count,
            airline: self.airline / count,
            flow_and_ground: self.flow_and_ground / count,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn minutes(value: i32) -> SimDuration {
        SimDuration::from_minutes(value)
    }

    #[test]
    fn test_split_charges_a_late_aircraft_first() {
        // Ready 40 minutes late, 10 drawn, left 50 late: 40 + 0 + 10.
        let causes = DelayCauses::split(minutes(50), minutes(40), minutes(10));
        assert_eq!(causes.late_aircraft(), minutes(40));
        assert_eq!(causes.airline(), SimDuration::ZERO);
        assert_eq!(causes.flow_and_ground(), minutes(10));
    }

    #[test]
    fn test_split_charges_the_airline_beyond_the_late_aircraft() {
        // Ready 5 late, 20 drawn, left 30 late: 5 + 15 + 10.
        let causes = DelayCauses::split(minutes(30), minutes(5), minutes(20));
        assert_eq!(causes.late_aircraft(), minutes(5));
        assert_eq!(causes.airline(), minutes(15));
        assert_eq!(causes.flow_and_ground(), minutes(10));
    }

    #[test]
    fn test_split_never_charges_more_than_the_delay() {
        // Ready early (negative lateness), 20 drawn, left only 12 late.
        let causes = DelayCauses::split(minutes(12), minutes(-30), minutes(20));
        assert_eq!(causes.late_aircraft(), SimDuration::ZERO);
        assert_eq!(causes.airline(), minutes(12));
        assert_eq!(causes.total(), minutes(12));
    }

    #[test]
    fn test_averages_and_share() {
        let mut total = DelayCauses::default();
        total.add(DelayCauses::split(minutes(30), minutes(20), minutes(0)));
        total.add(DelayCauses::split(minutes(10), minutes(0), minutes(10)));
        let average = total.per(2);
        assert_eq!(average.late_aircraft(), minutes(10));
        assert_eq!(average.airline(), minutes(5));
        assert_eq!(average.flow_and_ground(), minutes(5));
        assert!((total.late_aircraft_share() - 0.5).abs() < 1e-12);
        assert!(DelayCauses::default().late_aircraft_share().abs() < 1e-12);
        assert_eq!(DelayCauses::default().per(0), DelayCauses::default());
    }

    #[test]
    fn test_references_lie_in_the_acceptance_range() {
        // Plan M8 D172: late aircraft 35-50 % of departure delay.
        for (source, share) in LATE_AIRCRAFT_REFERENCES {
            assert!((0.35..=0.50).contains(&share), "{source}: {share}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_split_adds_up_to_the_delay(delay in 0_i64..86_400_000, late in -86_400_000_i64..86_400_000, drawn in 0_i64..3_600_000) {
            let causes = DelayCauses::split(
                SimDuration::from_millis(delay),
                SimDuration::from_millis(late),
                SimDuration::from_millis(drawn),
            );
            prop_assert_eq!(causes.total().as_millis(), delay);
            prop_assert!(causes.late_aircraft() >= SimDuration::ZERO);
            prop_assert!(causes.airline() >= SimDuration::ZERO);
            prop_assert!(causes.flow_and_ground() >= SimDuration::ZERO);
        }
    }
}
