//! Normalising `OurAirports` free-text runway surfaces.

use aerocosm_core::world::Surface;

/// Maps the many spellings in the data (`ASP`, `asphalt`, `Gras`, `GRS`, …) to
/// a [`Surface`]. Paved wins over grass for mixed values like `ASPH-G`.
pub(super) fn normalize_surface(text: &str) -> Surface {
    let text = text.trim().to_ascii_lowercase();
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| text.starts_with(p));
    if starts(&["asp", "con", "pem", "bit", "tar", "pav", "mac"]) {
        Surface::Paved
    } else if text == "g" || starts(&["gr", "turf"]) && !starts(&["grv", "grav"]) {
        Surface::Grass
    } else if starts(&[
        "grv", "grav", "dirt", "sand", "earth", "clay", "soil", "san",
    ]) {
        Surface::Unpaved
    } else if starts(&["wat"]) {
        Surface::Water
    } else {
        Surface::Unknown
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_normalize_surface_common_spellings() {
        let cases = [
            ("ASP", Surface::Paved),
            ("asphalt", Surface::Paved),
            ("ASPH-G", Surface::Paved),
            ("CON", Surface::Paved),
            ("concrete", Surface::Paved),
            ("PEM", Surface::Paved),
            ("BIT", Surface::Paved),
            ("GRS", Surface::Grass),
            ("Gras", Surface::Grass),
            ("GRASS", Surface::Grass),
            ("G", Surface::Grass),
            ("turf", Surface::Grass),
            ("GRV", Surface::Unpaved),
            ("gravel", Surface::Unpaved),
            ("dirt", Surface::Unpaved),
            ("sand", Surface::Unpaved),
            ("WATER", Surface::Water),
            ("UNK", Surface::Unknown),
            ("", Surface::Unknown),
        ];
        for (text, expected) in cases {
            assert_eq!(normalize_surface(text), expected, "{text:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_normalize_surface_ignores_case_and_whitespace(text in "[A-Za-z -]{0,12}") {
            prop_assert_eq!(normalize_surface(&text), normalize_surface(&format!("  {}  ", text.to_uppercase())));
        }
    }
}
