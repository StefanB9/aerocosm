//! One run of the suite as recorded in `benchmarks/history.jsonl` (plan M5
//! D141): when, where and on which commit it ran, and every result.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::parse::Sample;

/// The record format; a change that old records cannot be read with bumps it.
pub(crate) const FORMAT: u32 = 1;

/// The machine a run measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Machine {
    pub(crate) cpu: String,
    pub(crate) cores: usize,
    pub(crate) memory_mib: u64,
    pub(crate) os: String,
}

impl Machine {
    /// The machine as `/proc/cpuinfo` and `/proc/meminfo` describe it;
    /// what they lack is unknown.
    pub(crate) fn from_proc(cpuinfo: &str, meminfo: &str, os: &str) -> Self {
        let cpu = cpuinfo
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim() == "model name").then(|| value.trim().to_owned())
            })
            .unwrap_or_else(|| "unknown".to_owned());
        let cores = cpuinfo
            .lines()
            .filter(|line| {
                line.split(':')
                    .next()
                    .is_some_and(|k| k.trim() == "processor")
            })
            .count();
        let memory_mib = meminfo
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .and_then(|rest| rest.trim().strip_suffix("kB"))
            .and_then(|kib| kib.trim().parse::<u64>().ok())
            .map_or(0, |kib| kib / 1024);
        Self {
            cpu,
            cores,
            memory_mib,
            os: os.to_owned(),
        }
    }

    /// Whether results from `other` compare with this machine's.
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.cpu == other.cpu && self.cores == other.cores
    }
}

/// One result over the runs of a benchmark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Entry {
    /// The median over the runs, the lowest and highest, and for a single
    /// Criterion run its spread.
    Number {
        value: f64,
        unit: String,
        min: f64,
        max: f64,
        runs: usize,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        spread: Option<f64>,
    },
    /// A fingerprint, the same in every run.
    Hash { hash: String },
}

/// One run of the suite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) format: u32,
    /// Seconds since the Unix epoch when the run finished.
    pub(crate) timestamp: u64,
    pub(crate) commit: String,
    pub(crate) commit_date: String,
    pub(crate) toolchain: String,
    pub(crate) machine: Machine,
    /// The groups run.
    pub(crate) groups: Vec<String>,
    pub(crate) results: BTreeMap<String, Entry>,
}

impl Record {
    /// The record as one line of JSON, with its newline.
    pub(crate) fn to_line(&self) -> Result<String> {
        let mut line = serde_json::to_string(self).context("cannot write the record")?;
        line.push('\n');
        Ok(line)
    }
}

/// The records of a history file, oldest first.
pub(crate) fn read_history(text: &str) -> Result<Vec<Record>> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(number, line)| {
            let record: Record = serde_json::from_str(line)
                .with_context(|| format!("history line {}", number + 1))?;
            if record.format != FORMAT {
                bail!("history line {}: format {}", number + 1, record.format);
            }
            Ok(record)
        })
        .collect()
}

/// The results of several runs, one entry per name: the median of each
/// number and its range; a fingerprint that differs between runs is an
/// error, since the runs should be identical.
pub(crate) fn combine(runs: &[Vec<Sample>]) -> Result<BTreeMap<String, Entry>> {
    let mut numbers: BTreeMap<String, (String, Vec<f64>, Option<f64>)> = BTreeMap::new();
    let mut hashes: BTreeMap<String, String> = BTreeMap::new();
    for sample in runs.iter().flatten() {
        match sample {
            Sample::Number {
                name,
                value,
                unit,
                spread,
            } => {
                let entry = numbers
                    .entry(name.clone())
                    .or_insert_with(|| (unit.clone(), Vec::new(), *spread));
                entry.1.push(*value);
            }
            Sample::Hash { name, hash } => match hashes.get(name) {
                Some(seen) if seen != hash => {
                    bail!(
                        "{name} differs between runs ({seen}, {hash}): the run is not \
                         deterministic"
                    )
                }
                _ => {
                    hashes.insert(name.clone(), hash.clone());
                }
            },
        }
    }
    let mut results = BTreeMap::new();
    for (name, (unit, mut values, spread)) in numbers {
        values.sort_by(f64::total_cmp);
        let runs = values.len();
        let (Some(&min), Some(&max)) = (values.first(), values.last()) else {
            continue;
        };
        let value = if runs % 2 == 1 {
            values[runs / 2]
        } else {
            f64::midpoint(values[runs / 2 - 1], values[runs / 2])
        };
        results.insert(
            name,
            Entry::Number {
                value,
                unit,
                min,
                max,
                runs,
                spread: if runs == 1 { spread } else { None },
            },
        );
    }
    for (name, hash) in hashes {
        results.insert(name, Entry::Hash { hash });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    fn number(name: &str, value: f64) -> Sample {
        Sample::Number {
            name: name.to_owned(),
            value,
            unit: "ms".to_owned(),
            spread: None,
        }
    }

    fn hash(name: &str, hash: &str) -> Sample {
        Sample::Hash {
            name: name.to_owned(),
            hash: hash.to_owned(),
        }
    }

    #[test]
    fn test_machine_from_proc() {
        let cpuinfo = "processor\t: 0\nmodel name\t: Example CPU @ 3.0GHz\n\nprocessor\t: \
                       1\nmodel name\t: Example CPU @ 3.0GHz\n";
        let machine = Machine::from_proc(cpuinfo, "MemTotal:       16384000 kB\n", "linux");
        assert_eq!(
            machine,
            Machine {
                cpu: "Example CPU @ 3.0GHz".to_owned(),
                cores: 2,
                memory_mib: 16_000,
                os: "linux".to_owned()
            }
        );
        let unknown = Machine::from_proc("", "", "macos");
        assert_eq!(unknown.cpu, "unknown");
        assert!(!machine.same_as(&unknown));
        assert!(machine.same_as(&Machine {
            memory_mib: 1,
            ..machine.clone()
        }));
    }

    #[test]
    fn test_runs_combine_to_median_and_range() -> Result<()> {
        let runs = [
            vec![number("t", 3.0), hash("f", "ab")],
            vec![number("t", 1.0), hash("f", "ab")],
            vec![number("t", 2.0), hash("f", "ab"), number("only", 7.0)],
        ];
        let results = combine(&runs)?;
        assert_eq!(
            results.get("t"),
            Some(&Entry::Number {
                value: 2.0,
                unit: "ms".to_owned(),
                min: 1.0,
                max: 3.0,
                runs: 3,
                spread: None
            })
        );
        assert_eq!(
            results.get("f"),
            Some(&Entry::Hash {
                hash: "ab".to_owned()
            })
        );
        let only = results.get("only").context("only")?;
        assert!(matches!(only, Entry::Number { runs: 1, .. }));
        let even = combine(&[vec![number("t", 1.0)], vec![number("t", 2.0)]])?;
        assert!(
            matches!(even.get("t"), Some(Entry::Number { value, .. }) if (*value - 1.5).abs() < 1e-12)
        );
        assert!(combine(&[vec![hash("f", "ab")], vec![hash("f", "cd")]]).is_err());
        Ok(())
    }

    #[test]
    fn test_records_round_trip_through_the_history() -> Result<()> {
        let record = Record {
            format: FORMAT,
            timestamp: 1_763_200_000,
            commit: "abc123".to_owned(),
            commit_date: "2026-11-15T10:00:00+00:00".to_owned(),
            toolchain: "rustc 1.95.0-nightly".to_owned(),
            machine: Machine::from_proc("processor : 0\nmodel name : X\n", "", "linux"),
            groups: vec!["world".to_owned()],
            results: combine(&[vec![number("t", 1.5), hash("f", "ab")]])?,
        };
        let text = format!("{}\n{}", record.to_line()?, record.to_line()?);
        let history = read_history(&text)?;
        assert_eq!(history, [record.clone(), record]);
        assert!(read_history("{\"format\": 99}").is_err());
        assert!(read_history("not json").is_err());
        Ok(())
    }
}
