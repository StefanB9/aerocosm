//! ICAO wake turbulence separation (plan M3 D56).

use crate::{SimDuration, WakeCategory};

/// Metres per nautical mile.
pub(crate) const NAUTICAL_MILE: f64 = 1_852.0;
/// A landing aircraft occupies the runway about this long, so landings are
/// never closer together: the 50 s under which the reduced minimum on
/// final applies (plan M7 D159).
pub(crate) const RUNWAY_OCCUPANCY: SimDuration = SimDuration::from_secs(50);

/// Minimum distance on final between a `leader` and its `follower`, metres
/// (ICAO PANS-ATM wake turbulence minima; otherwise the reduced radar
/// minimum of 2.5 NM on final that busy runways fly, plan M7 D159).
pub(crate) fn arrival_distance(leader: WakeCategory, follower: WakeCategory) -> f64 {
    use WakeCategory::{Heavy, Light, Medium, Super};
    let miles = match (leader, follower) {
        (Super, Light) => 8.0,
        (Super, Medium) => 7.0,
        (Super, Heavy) | (Heavy, Light) => 6.0,
        (Heavy, Medium) | (Medium, Light) => 5.0,
        (Super | Heavy, Heavy | Super) => 4.0,
        _ => 2.5,
    };
    miles * NAUTICAL_MILE
}

/// Minimum time between two landings on one runway: the distance minimum at
/// the follower's `approach_speed` (m/s), never less than runway occupancy.
pub(crate) fn arrival_spacing(
    leader: WakeCategory,
    follower: WakeCategory,
    approach_speed: f64,
) -> SimDuration {
    let seconds = arrival_distance(leader, follower) / approach_speed.max(1.0);
    // Spacings are minutes, far inside i64 milliseconds.
    #[allow(clippy::cast_possible_truncation)]
    let spacing = SimDuration::from_millis((seconds * 1000.0).ceil() as i64);
    spacing.max(RUNWAY_OCCUPANCY)
}

/// Minimum time between two take-offs from one runway.
pub(crate) fn departure_spacing(leader: WakeCategory, follower: WakeCategory) -> SimDuration {
    use WakeCategory::{Heavy, Light, Medium, Super};
    let seconds = match (leader, follower) {
        (Super, Medium | Light) => 180,
        (Super, Heavy) | (Heavy, Medium | Light) => 120,
        _ => 60,
    };
    SimDuration::from_secs(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATEGORIES: [WakeCategory; 4] = [
        WakeCategory::Light,
        WakeCategory::Medium,
        WakeCategory::Heavy,
        WakeCategory::Super,
    ];

    #[test]
    fn test_arrival_distances_follow_icao() {
        let nm = |leader, follower| arrival_distance(leader, follower) / NAUTICAL_MILE;
        assert!((nm(WakeCategory::Heavy, WakeCategory::Medium) - 5.0).abs() < 1e-9);
        assert!((nm(WakeCategory::Heavy, WakeCategory::Heavy) - 4.0).abs() < 1e-9);
        assert!((nm(WakeCategory::Super, WakeCategory::Light) - 8.0).abs() < 1e-9);
        assert!((nm(WakeCategory::Medium, WakeCategory::Light) - 5.0).abs() < 1e-9);
        // The reduced radar minimum on final where no wake minimum applies
        // (ICAO PANS-ATM, runway occupancy under 50 s; plan M7 D159).
        assert!((nm(WakeCategory::Medium, WakeCategory::Medium) - 2.5).abs() < 1e-9);
        assert!((nm(WakeCategory::Light, WakeCategory::Heavy) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn test_arrival_spacing_converts_at_approach_speed_with_occupancy_floor() {
        // 5 NM at 70 m/s: 132 s.
        let spacing = arrival_spacing(WakeCategory::Heavy, WakeCategory::Medium, 70.0);
        assert!((spacing.as_secs_f64() - 5.0 * NAUTICAL_MILE / 70.0).abs() < 1.0);
        // 2.5 NM at a fast 200 m/s is 23 s, but never below the runway
        // occupancy of 50 s.
        let fast = arrival_spacing(WakeCategory::Medium, WakeCategory::Medium, 200.0);
        assert_eq!(fast, RUNWAY_OCCUPANCY);
        assert_eq!(RUNWAY_OCCUPANCY, SimDuration::from_secs(50));
    }

    #[test]
    fn test_departure_spacing_by_category() {
        let secs = |leader, follower| departure_spacing(leader, follower).as_secs_f64();
        assert!((secs(WakeCategory::Medium, WakeCategory::Medium) - 60.0).abs() < 1e-9);
        assert!((secs(WakeCategory::Heavy, WakeCategory::Medium) - 120.0).abs() < 1e-9);
        assert!((secs(WakeCategory::Heavy, WakeCategory::Heavy) - 60.0).abs() < 1e-9);
        assert!((secs(WakeCategory::Super, WakeCategory::Heavy) - 120.0).abs() < 1e-9);
        assert!((secs(WakeCategory::Super, WakeCategory::Medium) - 180.0).abs() < 1e-9);
    }

    #[test]
    fn test_spacing_never_shrinks_behind_heavier_leaders() {
        for follower in CATEGORIES {
            for pair in CATEGORIES.windows(2) {
                assert!(arrival_distance(pair[0], follower) <= arrival_distance(pair[1], follower));
                assert!(
                    departure_spacing(pair[0], follower) <= departure_spacing(pair[1], follower)
                );
            }
        }
    }
}
