//! Whether two parked aircraft keep clear of each other (plan M6 D148): a
//! stand is free for an aircraft if the aircraft actually parked on its
//! neighbours leave it room, so a wide-body takes the gates beside it and
//! two narrow-bodies fit where the mapping allows.

/// ICAO aerodrome code wingspans and stand wingtip clearances (Annex 14),
/// m: codes A to F.
pub const CODES: [(f64, f64); 6] = [
    (15.0, 3.0),
    (24.0, 3.0),
    (36.0, 4.5),
    (52.0, 7.5),
    (65.0, 7.5),
    (80.0, 7.5),
];
/// Stands mapped up to 5 % closer than the clearance asks still count.
pub const MAPPING_TOLERANCE: f64 = 1.05;

/// The wingtip clearance on a stand for an aircraft of `wingspan`, m.
pub fn wingtip_clearance(wingspan: f64) -> f64 {
    CODES
        .iter()
        .find(|(span, _)| wingspan <= *span)
        .map_or(CODES[CODES.len() - 1].1, |(_, clearance)| *clearance)
}

/// A parked aircraft: the stand point (its nose), east and north in metres,
/// its true heading in degrees and its wingspan in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parked {
    /// The stand point, m east and north.
    pub at: [f64; 2],
    /// True heading, degrees.
    pub heading: f64,
    /// Wingspan, m.
    pub wingspan: f64,
}

impl Parked {
    /// The centre of its footprint: a square as long as its wingspan (true
    /// to about a tenth for airliners) reaching back from the stand point.
    fn centre(&self) -> [f64; 2] {
        let (sin, cos) = self.heading.to_radians().sin_cos();
        let half = self.wingspan / 2.0;
        [self.at[0] - sin * half, self.at[1] - cos * half]
    }

    /// The distance to `other`'s footprint centre along its own axes (the
    /// larger of the two), m.
    fn gap_to(&self, other: &Self) -> f64 {
        let (sin, cos) = self.heading.to_radians().sin_cos();
        let (mine, theirs) = (self.centre(), other.centre());
        let delta = [theirs[0] - mine[0], theirs[1] - mine[1]];
        let along = delta[0] * sin + delta[1] * cos;
        let across = delta[0] * cos - delta[1] * sin;
        along.abs().max(across.abs())
    }

    /// Whether it and `other` keep the wingtip clearance of the larger of
    /// the two, allowing for mapping error: stands side by side are judged
    /// by their spacing, rows parked nose to nose by the gap between the
    /// noses. Two squares are apart if they are along an axis of either.
    pub fn keeps_clear_of(&self, other: &Self) -> bool {
        let larger = self.wingspan.max(other.wingspan);
        self.gap_to(other).max(other.gap_to(self)) * MAPPING_TOLERANCE
            >= f64::midpoint(self.wingspan, other.wingspan) + wingtip_clearance(larger)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn parked(x: f64, heading: f64, wingspan: f64) -> Parked {
        Parked {
            at: [x, 0.0],
            heading,
            wingspan,
        }
    }

    #[test]
    fn test_clearance_follows_the_code() {
        assert!((wingtip_clearance(23.0) - 3.0).abs() < 1e-9);
        assert!((wingtip_clearance(35.8) - 4.5).abs() < 1e-9);
        assert!((wingtip_clearance(36.1) - 7.5).abs() < 1e-9);
        assert!((wingtip_clearance(95.0) - 7.5).abs() < 1e-9);
    }

    #[test]
    fn test_neighbours_keep_clear_by_their_actual_sizes() {
        // Gates 38 m apart, built for the 737 (35.8 m): two fit side by
        // side within the mapping tolerance only just not (40.3 m asked,
        // 39.9 m there), an A320 (34.1 m) beside one does.
        let b737 = parked(0.0, 0.0, 35.8);
        assert!(!b737.keeps_clear_of(&parked(38.0, 0.0, 35.8)));
        assert!(b737.keeps_clear_of(&parked(38.0, 0.0, 34.1)));
        assert!(b737.keeps_clear_of(&parked(39.0, 0.0, 35.8)));
        // A wide-body (64.8 m) takes the gate beside it, not the next.
        let wide = parked(0.0, 0.0, 64.8);
        assert!(!wide.keeps_clear_of(&parked(38.0, 0.0, 35.8)));
        assert!(wide.keeps_clear_of(&parked(76.0, 0.0, 35.8)));
        // Either way round.
        assert!(!parked(38.0, 0.0, 35.8).keeps_clear_of(&wide));
        // A small aircraft needs less room.
        assert!(parked(0.0, 0.0, 24.0).keeps_clear_of(&parked(26.0, 0.0, 24.0)));
    }

    #[test]
    fn test_rows_nose_to_nose_are_judged_by_the_gap_between_noses() {
        // Facing each other 10 m apart: the footprints reach back, clear.
        let north = parked(0.0, 0.0, 35.8);
        let facing = Parked {
            at: [0.0, 10.0],
            heading: 180.0,
            wingspan: 35.8,
        };
        assert!(north.keeps_clear_of(&facing));
        // One behind the other in line needs a length and the clearance.
        let behind = |gap: f64| Parked {
            at: [0.0, -gap],
            heading: 0.0,
            wingspan: 35.8,
        };
        assert!(!north.keeps_clear_of(&behind(30.0)));
        assert!(north.keeps_clear_of(&behind(40.0)));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Clearance is mutual, and an aircraft never clears one on its
        /// own stand point.
        #[test]
        fn test_clearance_is_mutual(
            x in -200.0_f64..200.0, y in -200.0_f64..200.0,
            h1 in 0.0_f64..360.0, h2 in 0.0_f64..360.0,
            w1 in 15.0_f64..80.0, w2 in 15.0_f64..80.0,
        ) {
            let a = Parked { at: [0.0, 0.0], heading: h1, wingspan: w1 };
            let b = Parked { at: [x, y], heading: h2, wingspan: w2 };
            prop_assert_eq!(a.keeps_clear_of(&b), b.keeps_clear_of(&a));
            let same = Parked { at: [0.0, 0.0], heading: h1, wingspan: w2 };
            prop_assert!(!a.keeps_clear_of(&same));
            // Far enough apart everything is clear.
            let far = Parked { at: [x + 400.0, y], heading: h2, wingspan: w2 };
            prop_assert!(a.keeps_clear_of(&far));
        }
    }
}
