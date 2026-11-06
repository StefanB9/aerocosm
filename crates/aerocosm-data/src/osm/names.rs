//! OSM names and refs → [`GroundName`]s.

use aerocosm_core::ground::GroundName;

/// Longest ground name.
const MAX_LENGTH: usize = 16;

/// `text` as a ground name: German letters transliterated, other characters
/// outside printable ASCII dropped, runs of spaces collapsed, trimmed and cut
/// to 16 characters; `None` if nothing is left.
pub(crate) fn ground_name(text: &str) -> Option<GroundName> {
    let mut out = String::with_capacity(text.len().min(MAX_LENGTH * 2));
    for c in text.chars() {
        let replacement = match c {
            'ä' => "ae",
            'ö' => "oe",
            'ü' => "ue",
            'Ä' => "Ae",
            'Ö' => "Oe",
            'Ü' => "Ue",
            'ß' => "ss",
            c if c.is_whitespace() => " ",
            c if (' '..='~').contains(&c) => {
                out.push(c);
                continue;
            }
            _ => continue,
        };
        out.push_str(replacement);
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(MAX_LENGTH).collect();
    cut.trim_end().parse().ok()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn name(text: &str) -> Option<String> {
        ground_name(text).map(|n| n.to_string())
    }

    #[test]
    fn test_ground_name_cleans_osm_text() {
        assert_eq!(name("V 117"), Some("V 117".to_owned()));
        assert_eq!(name("  N7 "), Some("N7".to_owned()));
        assert_eq!(name("Vorfeld Süd"), Some("Vorfeld Sued".to_owned()));
        assert_eq!(name("Terminal\t 1"), Some("Terminal 1".to_owned()));
        assert_eq!(
            name("Start- und Landebahn Nordwest"),
            Some("Start- und Lande".to_owned())
        );
        assert_eq!(name("✈"), None);
        assert_eq!(name(""), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_ground_name_never_fails_on_text(text in "\\PC{0,40}") {
            if let Some(name) = ground_name(&text) {
                prop_assert!(!name.as_str().is_empty());
                prop_assert!(name.as_str().len() <= MAX_LENGTH);
            }
        }
    }
}
