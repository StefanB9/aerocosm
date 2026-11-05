//! Level of detail: what is drawn at which zoom.
//!
//! Zoom is the camera scale in map units (Web Mercator metres) per pixel.
//! Germany fills an 800 px tall window at ~1600; runways become recognisable
//! below ~60.

use aerocosm_core::world::AirportKind;

/// Below this scale runways are drawn to scale.
const RUNWAY_SCALE: f32 = 60.0;

/// Whether an airport symbol is drawn.
pub(crate) fn airport_visible(kind: AirportKind, scale: f32) -> bool {
    match kind {
        AirportKind::Major | AirportKind::Edge => true,
        AirportKind::Regional => scale < 1000.0,
        AirportKind::Airfield => scale < 250.0,
    }
}

/// Whether an airport's label is a candidate for placement.
pub(crate) fn airport_label_visible(kind: AirportKind, scale: f32) -> bool {
    let limit = match kind {
        AirportKind::Major | AirportKind::Edge => 2500.0,
        AirportKind::Regional => 400.0,
        AirportKind::Airfield => 60.0,
    };
    scale < limit
}

/// Whether a settlement label (Natural Earth rank, lower is bigger) is a
/// candidate.
pub(crate) fn place_label_visible(rank: u8, scale: f32) -> bool {
    let limit = match rank {
        0..=1 => f32::INFINITY,
        2..=3 => 1800.0,
        4..=5 => 900.0,
        6..=7 => 350.0,
        _ => 120.0,
    };
    scale < limit
}

/// Whether a river (Natural Earth rank) is drawn.
pub(crate) fn river_visible(rank: u8, scale: f32) -> bool {
    let limit = match rank {
        0..=4 => f32::INFINITY,
        5..=6 => 900.0,
        _ => 350.0,
    };
    scale < limit
}

/// Whether runways are drawn to scale.
pub(crate) fn runways_visible(scale: f32) -> bool {
    scale < RUNWAY_SCALE
}

/// On-screen radius of an airport symbol, in pixels.
pub(crate) fn glyph_radius(kind: AirportKind) -> f32 {
    match kind {
        AirportKind::Major => 6.0,
        AirportKind::Edge => 5.0,
        AirportKind::Regional => 4.5,
        AirportKind::Airfield => 3.0,
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::world::AirportKind;
    use proptest::prelude::*;

    use super::*;

    const KINDS: [AirportKind; 4] = [
        AirportKind::Major,
        AirportKind::Regional,
        AirportKind::Airfield,
        AirportKind::Edge,
    ];
    const COUNTRY_VIEW: f32 = 1600.0;

    #[test]
    fn test_country_view_shows_major_airports_and_big_cities_only() {
        assert!(airport_visible(AirportKind::Major, COUNTRY_VIEW));
        assert!(!airport_visible(AirportKind::Airfield, COUNTRY_VIEW));
        assert!(airport_label_visible(AirportKind::Major, COUNTRY_VIEW));
        assert!(!airport_label_visible(AirportKind::Regional, COUNTRY_VIEW));
        assert!(place_label_visible(1, COUNTRY_VIEW));
        assert!(!place_label_visible(8, COUNTRY_VIEW));
        assert!(!runways_visible(COUNTRY_VIEW));
    }

    #[test]
    fn test_airport_zoom_shows_everything() {
        for kind in KINDS {
            assert!(airport_visible(kind, 20.0));
            assert!(airport_label_visible(kind, 20.0));
        }
        assert!(runways_visible(20.0));
    }

    #[test]
    fn test_glyph_radius_ranks_kinds() {
        assert!(glyph_radius(AirportKind::Major) > glyph_radius(AirportKind::Regional));
        assert!(glyph_radius(AirportKind::Regional) > glyph_radius(AirportKind::Airfield));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_visibility_is_monotonic_in_zoom(kind in 0_usize..4, rank in 0_u8..12, far in 0.5_f32..5000.0, closer in 0.0_f32..1.0) {
            let near = far * closer;
            let kind = KINDS[kind];
            prop_assert!(!airport_visible(kind, far) || airport_visible(kind, near));
            prop_assert!(!airport_label_visible(kind, far) || airport_label_visible(kind, near));
            prop_assert!(!place_label_visible(rank, far) || place_label_visible(rank, near));
            prop_assert!(!river_visible(rank, far) || river_visible(rank, near));
            prop_assert!(!runways_visible(far) || runways_visible(near));
        }

        #[test]
        fn test_labels_only_where_glyphs_are(kind in 0_usize..4, scale in 0.5_f32..5000.0) {
            let kind = KINDS[kind];
            prop_assert!(!airport_label_visible(kind, scale) || airport_visible(kind, scale));
        }
    }
}
