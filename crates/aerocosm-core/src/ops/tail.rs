//! Per-tail sequencing and operational randomness: which leg comes next,
//! when it leaves the gate, and how long it taxis.

use anyhow::{Context, Result};

use crate::{SimDuration, SimRng, SimTime, time::LocalDate};

/// The leg after `leg` in a rotation of `count` legs flown in the week
/// starting `week`; after the last leg the rotation repeats a week later.
pub(crate) fn next_leg(leg: usize, count: usize, week: LocalDate) -> Result<(usize, LocalDate)> {
    if leg + 1 < count {
        Ok((leg + 1, week))
    } else {
        Ok((0, week.add_days(7)?))
    }
}

/// Off-block time: the scheduled departure plus the drawn delay, but never
/// before the aircraft is ready, so late arrivals delay the next leg.
pub(crate) fn off_block(scheduled: SimTime, delay: SimDuration, ready: SimTime) -> Result<SimTime> {
    let planned = scheduled
        .checked_add(delay)
        .with_context(|| format!("{scheduled} + {delay} is out of range"))?;
    Ok(planned.max(ready))
}

fn seconds(rng: &mut SimRng, from: u32, spread: u32) -> Result<u32> {
    Ok(from + rng.below(spread + 1).context("empty range")?)
}

/// A departure delay: 65 % leave within 3 minutes, 30 % within 20, the rest
/// within an hour.
pub(crate) fn draw_delay(rng: &mut SimRng) -> Result<SimDuration> {
    let class = rng.next_f64();
    let secs = if class < 0.65 {
        seconds(rng, 0, 179)?
    } else if class < 0.95 {
        seconds(rng, 180, 1_020)?
    } else {
        seconds(rng, 1_200, 2_400)?
    };
    Ok(SimDuration::from_secs(
        i32::try_from(secs).context("delay seconds")?,
    ))
}

/// Taxi-out time in seconds: 4–14 minutes, up to 25 at major airports.
pub(crate) fn draw_taxi_out(rng: &mut SimRng, major: bool) -> Result<f64> {
    let base = seconds(rng, 240, 600)?;
    let extra = if major { seconds(rng, 120, 540)? } else { 0 };
    Ok(f64::from(base + extra))
}

/// Taxi-in time in seconds: 2–8 minutes, up to 15 at major airports.
pub(crate) fn draw_taxi_in(rng: &mut SimRng, major: bool) -> Result<f64> {
    let base = seconds(rng, 120, 360)?;
    let extra = if major { seconds(rng, 60, 360)? } else { 0 };
    Ok(f64::from(base + extra))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;
    use crate::{RngSeed, StreamId};

    #[test]
    fn test_next_leg_wraps_into_the_next_week() -> Result<()> {
        let monday = LocalDate::new(2026, 7, 6)?;
        assert_eq!(next_leg(2, 5, monday)?, (3, monday));
        assert_eq!(next_leg(4, 5, monday)?, (0, LocalDate::new(2026, 7, 13)?));
        Ok(())
    }

    #[test]
    fn test_off_block_waits_for_the_aircraft() -> Result<()> {
        let scheduled: SimTime = "2026-07-06T10:00:00Z".parse()?;
        let delay = SimDuration::from_minutes(5);
        let early: SimTime = "2026-07-06T09:00:00Z".parse()?;
        let late: SimTime = "2026-07-06T10:20:00Z".parse()?;
        assert_eq!(
            off_block(scheduled, delay, early)?,
            "2026-07-06T10:05:00Z".parse()?
        );
        assert_eq!(off_block(scheduled, delay, late)?, late);
        Ok(())
    }

    #[test]
    fn test_delays_are_mostly_small() -> Result<()> {
        let mut rng = RngSeed::new(1).stream(StreamId::Operations);
        let mut punctual = 0;
        for _ in 0..10_000 {
            let delay = draw_delay(&mut rng)?;
            assert!(delay >= SimDuration::ZERO && delay <= SimDuration::from_minutes(60));
            if delay < SimDuration::from_minutes(3) {
                punctual += 1;
            }
        }
        assert!((6_000..7_000).contains(&punctual), "{punctual}");
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_taxi_times_stay_in_range(seed in any::<u64>(), major in any::<bool>()) {
            let mut rng = RngSeed::new(seed).stream(StreamId::Operations);
            let out = draw_taxi_out(&mut rng, major).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let back = draw_taxi_in(&mut rng, major).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!((240.0..=1_500.0).contains(&out), "{}", out);
            prop_assert!((120.0..=900.0).contains(&back), "{}", back);
            if !major {
                prop_assert!(out <= 840.0 && back <= 480.0);
            }
        }
    }
}
