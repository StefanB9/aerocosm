//! Results as the benchmarks print them: `bench <name> <value> <unit>` lines
//! from `aerocosm bench` and the viewer's frames, and Criterion's `bencher`
//! format (`test <name> ... bench: <n> ns/iter (+/- <d>)`).

/// One printed result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Sample {
    /// A quantity, its unit, and Criterion's spread where given.
    Number {
        name: String,
        value: f64,
        unit: String,
        spread: Option<f64>,
    },
    /// A fingerprint.
    Hash { name: String, hash: String },
}

/// The `bench` lines of `output`, other lines ignored.
pub(crate) fn bench_lines(output: &str) -> Vec<Sample> {
    output
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != "bench" {
                return None;
            }
            let (name, value, unit) = (words.next()?, words.next()?, words.next()?);
            if words.next().is_some() {
                return None;
            }
            if unit == "hash" {
                return Some(Sample::Hash {
                    name: name.to_owned(),
                    hash: value.to_owned(),
                });
            }
            Some(Sample::Number {
                name: name.to_owned(),
                value: value.parse().ok()?,
                unit: unit.to_owned(),
                spread: None,
            })
        })
        .collect()
}

/// Criterion's `bencher` lines of `output`, names prefixed with `prefix`.
pub(crate) fn bencher_lines(output: &str, prefix: &str) -> Vec<Sample> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("test ")?;
            let (name, rest) = rest.split_once(" ... bench:")?;
            let (value, rest) = rest.trim_start().split_once(" ns/iter")?;
            let spread = rest
                .trim()
                .strip_prefix("(+/-")
                .and_then(|s| s.strip_suffix(')'))
                .and_then(|s| number(s.trim()));
            Some(Sample::Number {
                name: format!("{prefix}{}", name.trim()),
                value: number(value.trim())?,
                unit: "ns".to_owned(),
                spread,
            })
        })
        .collect()
}

/// A number Criterion prints, with thousands separators.
fn number(text: &str) -> Option<f64> {
    text.replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_lines_are_read_and_others_ignored() {
        let output = "2026-11-15T10:00:00Z  INFO aerocosm: benchmark started\nbench \
                      tick-12/tick_median 10.25 ms\nbench tick-12/fingerprint 00000000000000ab \
                      hash\nbench broken line\nbench tick-12/x notanumber ms\n";
        assert_eq!(
            bench_lines(output),
            [
                Sample::Number {
                    name: "tick-12/tick_median".to_owned(),
                    value: 10.25,
                    unit: "ms".to_owned(),
                    spread: None
                },
                Sample::Hash {
                    name: "tick-12/fingerprint".to_owned(),
                    hash: "00000000000000ab".to_owned()
                },
            ]
        );
    }

    #[test]
    fn test_bencher_lines_are_read_with_their_spread() {
        let output = "Benchmarking geo/distance_to\ntest geo/distance_to ... bench:          24 \
                      ns/iter (+/- 1)\ntest ground/route_frankfurt ... bench:     223,320 ns/iter \
                      (+/- 9,100)\ntest odd ... bench: x ns/iter\n";
        let samples = bencher_lines(output, "micro/core/");
        assert_eq!(samples.len(), 2);
        assert_eq!(
            samples[1],
            Sample::Number {
                name: "micro/core/ground/route_frankfurt".to_owned(),
                value: 223_320.0,
                unit: "ns".to_owned(),
                spread: Some(9_100.0)
            }
        );
        assert!(
            matches!(&samples[0], Sample::Number { name, .. } if name == "micro/core/geo/distance_to")
        );
    }
}
