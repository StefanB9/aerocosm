//! Timetable building blocks: block times, turnarounds, time-of-day profiles
//! and rounding.

use crate::{
    AircraftType, SimDuration, SimRng, SimTime, WakeCategory,
    time::Zone,
    units::{Meters, NauticalMiles},
};

/// Minutes added to cruise time for the slower climb and descent; with the
/// default taxi times, calibrated against scheduled block times on German
/// sectors.
const AIRBORNE_OVERHEAD_MINUTES: f64 = 17.0;

/// Taxi minutes a block time allows at its two ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TaxiMinutes {
    /// At the origin, pushback included.
    pub(crate) out: f64,
    /// At the destination.
    pub(crate) back: f64,
}

impl TaxiMinutes {
    /// Where no ground layout gives better figures: about 12 minutes out and
    /// 6 in.
    pub(crate) const DEFAULT: Self = Self {
        out: 12.0,
        back: 6.0,
    };
}
/// Block times are published in five-minute steps.
const BLOCK_STEP_MINUTES: f64 = 5.0;

/// Gate-to-gate time for a sector of `distance` flown by `aircraft` with
/// `taxi` minutes at its ends (plan M4 D73).
pub(crate) fn block_time(
    distance: Meters,
    aircraft: &AircraftType,
    taxi: TaxiMinutes,
) -> SimDuration {
    let nautical: NauticalMiles = distance.to_nautical_miles();
    let cruise_minutes = nautical.value() / aircraft.cruise_speed().value() * 60.0;
    let total = cruise_minutes + AIRBORNE_OVERHEAD_MINUTES + taxi.out + taxi.back;
    let minutes = (total / BLOCK_STEP_MINUTES).ceil() * BLOCK_STEP_MINUTES;
    // SAFETY: sectors inside the map are at most a few hours; the cast cannot
    // overflow.
    #[allow(clippy::cast_possible_truncation)]
    SimDuration::from_minutes(minutes as i32)
}

/// Shortest turnaround at the gate for `aircraft`, flying freight when
/// `cargo` (a passenger type converted to a freighter turns like one).
pub(crate) fn min_turnaround(aircraft: &AircraftType, cargo: bool) -> SimDuration {
    let minutes = if cargo || aircraft.is_freighter() {
        60
    } else if matches!(aircraft.wake(), WakeCategory::Heavy | WakeCategory::Super) {
        90
    } else if aircraft.seats() <= 100 {
        25
    } else {
        35
    };
    SimDuration::from_minutes(minutes)
}

/// Local time minus UTC at `at`, in minutes, within `-720..=720`.
pub(crate) fn offset_minutes(zone: &Zone, at: SimTime) -> i32 {
    let local = i32::from(zone.local(at).minute_of_day());
    let civil = at.to_civil();
    let utc = i32::from(civil.hour()) * 60 + i32::from(civil.minute());
    let mut offset = (local - utc).rem_euclid(1440);
    if offset > 720 {
        offset -= 1440;
    }
    offset
}

/// Index drawn with probability proportional to `weights`; `None` if no weight
/// is positive.
pub(crate) fn pick_weighted(rng: &mut SimRng, weights: &[f64]) -> Option<usize> {
    let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if total <= 0.0 {
        return None;
    }
    let mut remaining = rng.next_f64() * total;
    let mut last = None;
    for (index, weight) in weights.iter().enumerate() {
        if *weight <= 0.0 {
            continue;
        }
        last = Some(index);
        if remaining < *weight {
            return Some(index);
        }
        remaining -= weight;
    }
    // Rounding can leave a sliver at the end; it belongs to the last positive
    // weight.
    last
}

/// `floor(value)` or `ceil(value)`, rounding up with probability equal to the
/// fraction, so sums of many rounded values stay unbiased.
pub(crate) fn stochastic_round(rng: &mut SimRng, value: f64) -> u32 {
    let floor = value.max(0.0).floor();
    let up = rng.next_f64() < value - floor;
    // SAFETY: daily flight counts are far below u32::MAX.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let base = floor as u32;
    base + u32::from(up)
}

/// When in the local day departures happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    /// Passenger flights: morning and evening peaks, nothing in the small
    /// hours.
    PassengerDay,
    /// Cargo flights: late evening to early morning.
    CargoNight,
}

impl Profile {
    /// Relative departures per local hour, midnight first.
    pub(crate) const fn weights(self) -> [f64; 24] {
        match self {
            Self::PassengerDay => [
                0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 6.0, 9.0, 8.0, 6.0, 5.0, 5.0, 5.0, 5.0, 5.0, 6.0,
                7.0, 8.0, 8.0, 7.0, 5.0, 3.0, 1.0, 0.0,
            ],
            Self::CargoNight => [
                6.0, 6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 6.0, 8.0,
            ],
        }
    }
}

/// A local departure minute drawn from `profile`.
pub(crate) fn sample_minute(rng: &mut SimRng, profile: Profile) -> u16 {
    let hour = pick_weighted(rng, &profile.weights()).unwrap_or(12);
    // SAFETY: hour < 24 and below(60) < 60, so the minute is below 1440.
    #[allow(clippy::cast_possible_truncation)]
    let minute = rng.below(60).unwrap_or(0) as u16;
    #[allow(clippy::cast_possible_truncation)]
    let hour = hour as u16;
    hour * 60 + minute
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{AircraftCatalog, RngSeed, StreamId};

    fn catalog() -> Result<AircraftCatalog> {
        AircraftCatalog::from_json(include_str!("../../../../assets/aircraft/types.json"))
    }

    fn aircraft<'a>(catalog: &'a AircraftCatalog, code: &str) -> Result<&'a AircraftType> {
        catalog
            .by_code(code.parse()?)
            .and_then(|id| catalog.get(id))
            .context("type")
    }

    #[test]
    fn test_block_time_matches_real_sectors() -> Result<()> {
        let catalog = catalog()?;
        let a320 = aircraft(&catalog, "A320")?;
        // Frankfurt-Munich, 299 km: scheduled block ~55-65 min.
        let taxi = TaxiMinutes::DEFAULT;
        let short = block_time(Meters::new(299_000.0)?, a320, taxi).as_millis() / 60_000;
        assert!((50..=70).contains(&short), "{short} min");
        // London-Frankfurt, 655 km: scheduled block ~85-100 min.
        let medium = block_time(Meters::new(655_000.0)?, a320, taxi).as_millis() / 60_000;
        assert!((80..=105).contains(&medium), "{medium} min");
        assert_eq!(short % 5, 0, "rounded to five minutes");
        // Ten more minutes of taxiing add ten minutes, rounded.
        let longer = TaxiMinutes {
            out: taxi.out + 10.0,
            back: taxi.back,
        };
        let slow = block_time(Meters::new(299_000.0)?, a320, longer).as_millis() / 60_000;
        assert_eq!(slow, short + 10);
        Ok(())
    }

    #[test]
    fn test_min_turnaround_by_size() -> Result<()> {
        let catalog = catalog()?;
        let minutes = |code: &str| -> Result<i64> {
            Ok(min_turnaround(aircraft(&catalog, code)?, false).as_millis() / 60_000)
        };
        assert_eq!(minutes("CRJ9")?, 25);
        assert_eq!(minutes("A320")?, 35);
        assert_eq!(minutes("A359")?, 90);
        assert_eq!(minutes("B77L")?, 60, "freighters");
        let freight = min_turnaround(aircraft(&catalog, "B763")?, true);
        assert_eq!(freight.as_millis() / 60_000, 60, "converted freighters");
        Ok(())
    }

    #[test]
    fn test_offset_minutes_follows_daylight_saving() -> Result<()> {
        let winter: SimTime = "2026-01-15T12:00:00Z".parse()?;
        let summer: SimTime = "2026-07-15T12:00:00Z".parse()?;
        assert_eq!(offset_minutes(&Zone::new("Europe/Berlin")?, winter), 60);
        assert_eq!(offset_minutes(&Zone::new("Europe/Berlin")?, summer), 120);
        assert_eq!(offset_minutes(&Zone::new("Europe/London")?, winter), 0);
        assert_eq!(offset_minutes(&Zone::new("Europe/Istanbul")?, winter), 180);
        Ok(())
    }

    #[test]
    fn test_pick_weighted_skips_zero_weights() {
        let mut rng = RngSeed::new(1).stream(StreamId::Scheduling);
        for _ in 0..1000 {
            assert_eq!(pick_weighted(&mut rng, &[0.0, 2.0, 0.0]), Some(1));
        }
        assert_eq!(pick_weighted(&mut rng, &[0.0, 0.0]), None);
        assert_eq!(pick_weighted(&mut rng, &[]), None);
    }

    #[test]
    fn test_stochastic_round_is_unbiased() {
        let mut rng = RngSeed::new(2).stream(StreamId::Scheduling);
        let total: u32 = (0..10_000).map(|_| stochastic_round(&mut rng, 0.3)).sum();
        assert!((2_800..=3_200).contains(&total), "{total}");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_stochastic_round_is_floor_or_ceil(value in 0.0_f64..500.0, seed in any::<u64>()) {
            let mut rng = RngSeed::new(seed).stream(StreamId::Scheduling);
            let rounded = stochastic_round(&mut rng, value);
            // SAFETY: value < 500, so floor and ceil fit in u32.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (floor, ceil) = (value.floor() as u32, value.ceil() as u32);
            prop_assert!(rounded == floor || rounded == ceil);
        }

        #[test]
        fn test_profile_samples_only_open_hours(seed in any::<u64>()) {
            let mut rng = RngSeed::new(seed).stream(StreamId::Scheduling);
            for profile in [Profile::PassengerDay, Profile::CargoNight] {
                let minute = sample_minute(&mut rng, profile);
                let hour = usize::from(minute / 60);
                prop_assert!(minute < 1440);
                prop_assert!(profile.weights()[hour] > 0.0, "{profile:?} sampled hour {hour}");
            }
        }
    }
}
