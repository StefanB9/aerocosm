//! Hand-curated stand sizes (`assets/ground/stand-sizes.json`): where the
//! spacing of the mapped stands misjudges what an apron takes, published
//! limits bound the ICAO code the import derives (plan M4 D68).

use aerocosm_core::world::codes::AirportIdent;
use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::stands::code_wingspan;

/// The version of the stand size file this importer reads.
pub const SIZES_VERSION: u32 = 1;

/// An ICAO aerodrome code letter (Annex 14), by wingspan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum Code {
    /// Wingspan under 15 m.
    A,
    /// Under 24 m.
    B,
    /// Under 36 m (A320, B737).
    C,
    /// Under 52 m (B757, B767).
    D,
    /// Under 65 m (A330, B747-400, B777).
    E,
    /// Under 80 m (A380, B747-8).
    F,
}

impl Code {
    /// The largest wingspan of the code, m.
    fn wingspan(self) -> f64 {
        code_wingspan(self as usize)
    }
}

/// Size limits of the stands whose names start with one of `stands`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    stands: Vec<String>,
    #[serde(default)]
    at_least: Option<Code>,
    #[serde(default)]
    at_most: Option<Code>,
    /// Where the limit comes from.
    #[allow(dead_code, reason = "documentation in the file")]
    note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AirportSizes {
    airport: AirportIdent,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SizesFile {
    format_version: u32,
    #[allow(dead_code, reason = "documentation in the file")]
    source: String,
    airports: Vec<AirportSizes>,
}

/// Curated stand size limits of every airport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StandSizes {
    airports: Vec<AirportSizes>,
}

impl StandSizes {
    /// Reads the stand size file.
    pub fn from_json(text: &str) -> Result<Self> {
        let file: SizesFile = serde_json::from_str(text).context("invalid stand size file")?;
        if file.format_version != SIZES_VERSION {
            bail!(
                "stand size file version {} is not {SIZES_VERSION}",
                file.format_version
            );
        }
        for airport in &file.airports {
            for rule in &airport.rules {
                if rule.stands.iter().any(String::is_empty) {
                    bail!("{}: empty stand prefix", airport.airport);
                }
                if let (Some(low), Some(high)) = (rule.at_least, rule.at_most)
                    && low > high
                {
                    bail!(
                        "{}: {:?} is at least {low:?} but at most {high:?}",
                        airport.airport,
                        rule.stands
                    );
                }
            }
        }
        Ok(Self {
            airports: file.airports,
        })
    }

    /// The stand-name prefixes of `airport`'s rules.
    pub fn prefixes(&self, airport: AirportIdent) -> impl Iterator<Item = &str> {
        self.airports
            .iter()
            .filter(move |a| a.airport == airport)
            .flat_map(|a| &a.rules)
            .flat_map(|r| r.stands.iter().map(String::as_str))
    }

    /// `span`, the wingspan limit derived for `airport`'s stand `name`, m,
    /// bounded by the rule with the longest prefix of `name`.
    pub fn limit(&self, airport: AirportIdent, name: &str, span: f64) -> f64 {
        let best = self
            .airports
            .iter()
            .filter(|a| a.airport == airport)
            .flat_map(|a| &a.rules)
            .flat_map(|rule| rule.stands.iter().map(move |prefix| (prefix, rule)))
            .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len());
        let Some((_, rule)) = best else {
            return span;
        };
        let low = rule.at_least.map_or(span, |code| span.max(code.wingspan()));
        rule.at_most.map_or(low, |code| low.min(code.wingspan()))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    const SAMPLE: &str = r#"{
        "format_version": 1,
        "source": "test",
        "airports": [{
            "airport": "EDDK",
            "rules": [
                {"stands": ["A", "B"], "at_least": "C", "note": "piers"},
                {"stands": ["E"], "at_least": "E", "note": "cargo"},
                {"stands": ["E31"], "at_least": "F", "note": "A380"},
                {"stands": ["U1"], "at_most": "C", "note": "small"}
            ]
        }]
    }"#;

    #[test]
    fn test_stand_sizes_bound_the_derived_span_by_the_longest_prefix() -> Result<()> {
        let sizes = StandSizes::from_json(SAMPLE)?;
        let cgn: AirportIdent = "EDDK".parse()?;
        assert!((sizes.limit(cgn, "A10", 15.0) - 36.0).abs() < 1e-9);
        // An at-least rule keeps a larger derived span.
        assert!((sizes.limit(cgn, "B22", 52.0) - 52.0).abs() < 1e-9);
        assert!((sizes.limit(cgn, "E13", 15.0) - 65.0).abs() < 1e-9);
        assert!((sizes.limit(cgn, "E31", 15.0) - 80.0).abs() < 1e-9);
        assert!((sizes.limit(cgn, "U12", 52.0) - 36.0).abs() < 1e-9);
        // Unmatched stands and other airports keep the derived span.
        assert!((sizes.limit(cgn, "V10", 15.0) - 15.0).abs() < 1e-9);
        assert!((sizes.limit("EDDF".parse()?, "A10", 15.0) - 15.0).abs() < 1e-9);
        assert_eq!(sizes.prefixes(cgn).count(), 5);
        Ok(())
    }

    #[test]
    fn test_stand_sizes_reject_bad_files() {
        assert!(StandSizes::from_json("{}").is_err());
        assert!(
            StandSizes::from_json(
                &SAMPLE.replace("\"format_version\": 1", "\"format_version\": 2")
            )
            .is_err()
        );
        assert!(StandSizes::from_json(&SAMPLE.replace("[\"E\"]", "[\"\"]")).is_err());
        assert!(
            StandSizes::from_json(&SAMPLE.replace(
                "\"at_most\": \"C\"",
                "\"at_most\": \"C\", \"at_least\": \"D\""
            ))
            .is_err()
        );
        assert!(StandSizes::from_json(&SAMPLE.replace("\"C\"", "\"G\"")).is_err());
    }

    #[test]
    fn test_shipped_stand_sizes_match_mapped_stands() -> Result<()> {
        let sizes =
            StandSizes::from_json(include_str!("../../../../assets/ground/stand-sizes.json"))?;
        let world = aerocosm_core::world::World::from_asset(include_bytes!(
            "../../../../assets/world/world.bin"
        ))?;
        let ground = aerocosm_core::ground::GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        for airport in &sizes.airports {
            let id = world
                .airport_by_ident(airport.airport)
                .with_context(|| format!("{} is not in the world", airport.airport))?;
            let layout = ground
                .airport(id)
                .with_context(|| format!("{} has no layout", airport.airport))?;
            for prefix in sizes.prefixes(airport.airport) {
                let matched: Vec<_> = layout
                    .stands()
                    .iter()
                    .filter(|s| s.name().as_str().starts_with(prefix))
                    .collect();
                anyhow::ensure!(
                    !matched.is_empty(),
                    "{}: no stand {prefix}",
                    airport.airport
                );
                // The asset was regenerated with the rules applied.
                for stand in matched {
                    let span = stand.max_wingspan().value();
                    let bounded = sizes.limit(airport.airport, stand.name().as_str(), span);
                    anyhow::ensure!(
                        (bounded - span).abs() < 1e-6,
                        "{} {}: {span} m, rules say {bounded} m",
                        airport.airport,
                        stand.name()
                    );
                }
            }
        }
        Ok(())
    }
}
