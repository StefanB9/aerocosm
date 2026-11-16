//! `cargo xtask bench report`: the latest record against the previous one on
//! the same machine, one line per result (plan M5 D141). No thresholds yet:
//! those follow the first benchmark step.

use std::{collections::BTreeSet, fmt::Write as _};

use super::record::{Entry, Record};

/// The previous record on `latest`'s machine in `history` (oldest first),
/// not counting `latest` itself.
pub(crate) fn previous<'a>(history: &'a [Record], latest: &Record) -> Option<&'a Record> {
    history
        .iter()
        .rev()
        .filter(|record| *record != latest)
        .find(|record| record.machine.same_as(&latest.machine))
}

/// The comparison of `latest` with `earlier` as text.
pub(crate) fn compare(latest: &Record, earlier: Option<&Record>) -> String {
    let mut text = String::new();
    let short = |commit: &str| commit.chars().take(10).collect::<String>();
    let _ = writeln!(
        text,
        "{} ({}) on {}, {} cores",
        short(&latest.commit),
        latest.commit_date,
        latest.machine.cpu,
        latest.machine.cores
    );
    match earlier {
        Some(earlier) => {
            let _ = writeln!(
                text,
                "against {} ({})",
                short(&earlier.commit),
                earlier.commit_date
            );
        }
        None => {
            let _ = writeln!(text, "no earlier record on this machine");
        }
    }
    let names: BTreeSet<&String> = latest
        .results
        .keys()
        .chain(earlier.into_iter().flat_map(|e| e.results.keys()))
        .collect();
    let width = names.iter().map(|n| n.len()).max().unwrap_or(0);
    for name in names {
        let now = latest.results.get(name);
        let then = earlier.and_then(|e| e.results.get(name));
        let _ = writeln!(text, "{name:<width$}  {}", line(now, then));
    }
    text
}

fn line(now: Option<&Entry>, then: Option<&Entry>) -> String {
    match (now, then) {
        (
            Some(Entry::Number {
                value,
                unit,
                min,
                max,
                ..
            }),
            Some(Entry::Number {
                value: was,
                min: was_min,
                max: was_max,
                ..
            }),
        ) => {
            let change = if *was == 0.0 {
                String::new()
            } else {
                format!("{:+.1} %", (value / was - 1.0) * 100.0)
            };
            // Ranges over several runs that overlap: within the noise.
            let overlap = if min <= was_max && was_min <= max && (min < max || was_min < was_max) {
                "  (ranges overlap)"
            } else {
                ""
            };
            format!(
                "{} {unit}  (was {})  {change}{overlap}",
                figure(*value),
                figure(*was)
            )
        }
        (Some(Entry::Number { value, unit, .. }), _) => format!("{} {unit}  (new)", figure(*value)),
        (Some(Entry::Hash { hash }), Some(Entry::Hash { hash: was })) => {
            if hash == was {
                format!("{hash}  (same)")
            } else {
                format!("{hash}  (was {was}: behaviour changed)")
            }
        }
        (Some(Entry::Hash { hash }), _) => format!("{hash}  (new)"),
        (None, _) => "(gone)".to_owned(),
    }
}

/// A number with about four significant digits.
fn figure(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000.0 {
        format!("{value:.0}")
    } else if magnitude >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::bench::record::{FORMAT, Machine};

    fn record(commit: &str, cpu: &str, results: &[(&str, Entry)]) -> Record {
        Record {
            format: FORMAT,
            timestamp: 0,
            commit: commit.to_owned(),
            commit_date: "2026-11-15".to_owned(),
            toolchain: "rustc".to_owned(),
            machine: Machine {
                cpu: cpu.to_owned(),
                cores: 4,
                memory_mib: 1,
                os: "linux".to_owned(),
            },
            groups: Vec::new(),
            results: results
                .iter()
                .map(|(n, e)| ((*n).to_owned(), e.clone()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn ms(value: f64, min: f64, max: f64) -> Entry {
        Entry::Number {
            value,
            unit: "ms".to_owned(),
            min,
            max,
            runs: 3,
            spread: None,
        }
    }

    fn hash(hash: &str) -> Entry {
        Entry::Hash {
            hash: hash.to_owned(),
        }
    }

    #[test]
    fn test_previous_record_is_on_the_same_machine() {
        let a = record("a", "X", &[]);
        let b = record("b", "Y", &[]);
        let c = record("c", "X", &[]);
        let history = [a.clone(), b, c.clone()];
        assert_eq!(previous(&history, &c), Some(&a));
        assert_eq!(previous(&history[..1], &a), None);
    }

    #[test]
    fn test_comparison_lists_changes_per_result() {
        let earlier = record(
            "aaaaaaaaaaaaaaa",
            "X",
            &[
                ("tick", ms(10.0, 9.0, 11.0)),
                ("frame", ms(2.0, 2.0, 2.0)),
                ("print", hash("ab")),
                ("same", hash("cd")),
                ("old", ms(1.0, 1.0, 1.0)),
            ],
        );
        let latest = record(
            "bbbbbbbbbbbbbbb",
            "X",
            &[
                ("tick", ms(8.0, 7.5, 9.5)),
                ("frame", ms(3.0, 3.0, 3.0)),
                ("print", hash("ef")),
                ("same", hash("cd")),
                ("new", ms(5.0, 5.0, 5.0)),
            ],
        );
        let text = compare(&latest, Some(&earlier));
        assert!(text.starts_with("bbbbbbbbbb (2026-11-15) on X, 4 cores\nagainst aaaaaaaaaa"));
        assert!(
            text.contains("tick   8.000 ms  (was 10.00)  -20.0 %  (ranges overlap)"),
            "{text}"
        );
        assert!(
            text.contains("frame  3.000 ms  (was 2.000)  +50.0 %\n"),
            "{text}"
        );
        assert!(
            text.contains("print  ef  (was ab: behaviour changed)"),
            "{text}"
        );
        assert!(text.contains("same   cd  (same)"), "{text}");
        assert!(text.contains("new    5.000 ms  (new)"), "{text}");
        assert!(text.contains("old    (gone)"), "{text}");
        let alone = compare(&latest, None);
        assert!(alone.contains("no earlier record on this machine"));
        assert!(alone.contains("tick   8.000 ms  (new)"));
    }
}
