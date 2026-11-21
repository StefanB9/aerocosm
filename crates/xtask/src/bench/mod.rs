//! `cargo xtask bench` (plan M5 D141): builds and runs the benchmark suite,
//! appends one record per run to `benchmarks/history.jsonl` from a clean
//! working tree, and compares it with the previous record on the same
//! machine.

pub(crate) mod machine;
pub(crate) mod parse;
pub(crate) mod record;
pub(crate) mod report;

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tracing::info;

use self::{
    parse::{Sample, bench_lines, bencher_lines},
    record::{FORMAT, Record, combine, read_history},
};
use crate::host::Host;

/// The committed history.
pub(crate) const HISTORY: &str = "benchmarks/history.jsonl";
/// Every run's record, recorded or not, outside the repository.
pub(crate) const LAST_RUN: &str = "target/bench-runs.jsonl";
/// The runner the world scenarios run in, built in release.
const RUNNER: &str = "target/release/aerocosm";
/// The viewer the window group measures, built in release.
const VIEWER: &str = "target/release/aerocosm-viewer";
/// The hours whose ticks are measured (plan M5 D135).
const TICK_SCENARIOS: [&str; 4] = ["tick-00", "tick-06", "tick-12", "tick-16"];

/// A group of benchmarks, runnable alone (plan M5 D134).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    /// Decoding and setting up the world.
    Startup,
    /// Ticks at four hours of the day and a simulated day.
    World,
    /// Criterion benchmarks of the core and the viewer.
    Micro,
    /// The viewer's frames, headless.
    Viewer,
    /// The viewer's frames in a real window, render thread and GPU included
    /// (plan M9 D182); in the default run only where a display exists.
    Window,
}

impl Group {
    pub(crate) const ALL: [Self; 5] = [
        Self::Startup,
        Self::World,
        Self::Micro,
        Self::Viewer,
        Self::Window,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::World => "world",
            Self::Micro => "micro",
            Self::Viewer => "viewer",
            Self::Window => "window",
        }
    }
}

/// How to run the suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// The groups to run; all when empty.
    pub(crate) groups: Vec<Group>,
    /// Runs of each start-up and tick scenario.
    pub(crate) repeat: usize,
    /// A few ticks and frames only, to check that everything runs; never
    /// recorded.
    pub(crate) quick: bool,
    /// The cargo to build and bench with.
    pub(crate) cargo: String,
    /// Seconds since the Unix epoch, now.
    pub(crate) timestamp: u64,
    /// The operating system's name.
    pub(crate) os: String,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) record: Record,
    /// Whether it was appended to the history.
    pub(crate) recorded: bool,
}

fn strings(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// The runner's arguments for `scenario`.
fn scenario_args(scenario: &str, quick: bool) -> Vec<OsString> {
    let mut args = strings(&["bench", scenario]);
    if quick {
        let ticks = if scenario == "day" { "60" } else { "5" };
        args.extend(strings(&[
            "--settle", "2", "--ticks", ticks, "--split", "2",
        ]));
    }
    args
}

/// Criterion's arguments after `--`: the `bencher` output, shorter when
/// quick.
fn criterion_args(quick: bool) -> Vec<OsString> {
    let mut args = strings(&["--output-format", "bencher"]);
    if quick {
        args.extend(strings(&[
            "--warm-up-time",
            "0.1",
            "--measurement-time",
            "0.2",
        ]));
    }
    args
}

fn bench_args(package: &str, bench: &str, after: Vec<OsString>) -> Vec<OsString> {
    let mut args = strings(&[
        "bench",
        "-p",
        package,
        "--bench",
        bench,
        "--profile",
        "bench-unwind",
        "--",
    ]);
    args.extend(after);
    args
}

/// The runs of one group's benchmarks.
fn run_group(host: &mut impl Host, group: Group, options: &Options) -> Result<Vec<Vec<Sample>>> {
    let repeat = if options.quick {
        1
    } else {
        options.repeat.max(1)
    };
    let mut runs = Vec::new();
    match group {
        Group::Startup | Group::World => {
            let names: &[&str] = if group == Group::Startup {
                &["startup"]
            } else {
                &TICK_SCENARIOS
            };
            let mut scenario = |name: &str| -> Result<Vec<Sample>> {
                info!(scenario = name, "running");
                let output = host.output(RUNNER, &scenario_args(name, options.quick))?;
                Ok(bench_lines(&output))
            };
            for name in names {
                for _ in 0..repeat {
                    runs.push(scenario(name)?);
                }
            }
            if group == Group::World {
                runs.push(scenario("day")?);
            }
        }
        Group::Micro => {
            for (package, bench) in [("aerocosm-core", "core"), ("aerocosm-viewer", "micro")] {
                let output = host.output(
                    &options.cargo,
                    &bench_args(package, bench, criterion_args(options.quick)),
                )?;
                runs.push(bencher_lines(&output, "micro/"));
            }
        }
        Group::Window => {
            let mut args = strings(&["--measure"]);
            if options.quick {
                args.extend(strings(&["quick"]));
            }
            runs.push(bench_lines(&host.output(VIEWER, &args)?));
        }
        Group::Viewer => {
            let frames = if options.quick { "5" } else { "300" };
            let output = host.output(
                &options.cargo,
                &bench_args("aerocosm-viewer", "frames", strings(&[frames])),
            )?;
            runs.push(bench_lines(&output));
        }
    }
    Ok(runs)
}

/// Runs the suite and records it if the working tree is clean and the run
/// is not quick.
pub(crate) fn run(host: &mut impl Host, options: &Options) -> Result<Outcome> {
    let groups = if options.groups.is_empty() {
        let display = host.has_display();
        Group::ALL
            .into_iter()
            .filter(|group| *group != Group::Window || display)
            .collect()
    } else {
        let mut groups = options.groups.clone();
        groups.sort();
        groups.dedup();
        groups
    };
    let clean = host
        .output("git", &strings(&["status", "--porcelain"]))?
        .trim()
        .is_empty();
    let commit = host
        .output("git", &strings(&["rev-parse", "HEAD"]))?
        .trim()
        .to_owned();
    let commit_date = host
        .output("git", &strings(&["log", "-1", "--format=%cI"]))?
        .trim()
        .to_owned();
    let toolchain = host.output("rustc", &strings(&["-V"]))?.trim().to_owned();
    let machine = host.machine(&options.os);
    if groups.contains(&Group::Startup) || groups.contains(&Group::World) {
        host.run(
            &options.cargo,
            &strings(&["build", "--release", "-p", "aerocosm"]),
        )?;
    }
    if groups.contains(&Group::Window) {
        host.run(
            &options.cargo,
            &strings(&["build", "--release", "-p", "aerocosm-viewer"]),
        )?;
    }
    let mut runs = Vec::new();
    for group in &groups {
        info!(group = group.name(), "benchmarking");
        runs.extend(
            run_group(host, *group, options)
                .with_context(|| format!("benchmark group {}", group.name()))?,
        );
    }
    let record = Record {
        format: FORMAT,
        timestamp: options.timestamp,
        commit,
        commit_date,
        toolchain,
        machine,
        groups: groups.iter().map(|g| g.name().to_owned()).collect(),
        results: combine(&runs)?,
    };
    // Kept before the history is touched: a run takes half an hour.
    let line = record.to_line()?;
    host.append(Path::new(LAST_RUN), &line)?;
    let recorded = clean && !options.quick;
    if recorded {
        host.append(Path::new(HISTORY), &line)?;
    }
    Ok(Outcome { record, recorded })
}

/// The history in `path`; empty if there is none yet.
pub(crate) fn history(host: &mut impl Host, path: &Path) -> Result<Vec<Record>> {
    match host.read_to_string(path) {
        Ok(text) => read_history(&text),
        Err(_) => Ok(Vec::new()),
    }
}

/// The comparison of `latest` with the previous record on its machine.
pub(crate) fn compare_with_history(host: &mut impl Host, latest: &Record) -> Result<String> {
    let history = history(host, &PathBuf::from(HISTORY))?;
    Ok(report::compare(latest, report::previous(&history, latest)))
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};

    use super::{record::Machine, *};
    use crate::host::join_args;

    /// Answers with canned output and notes every call.
    struct FakeHost {
        dirty: bool,
        display: bool,
        fingerprint: Vec<&'static str>,
        calls: Vec<String>,
        appended: Vec<String>,
    }

    impl FakeHost {
        fn new(dirty: bool) -> Self {
            Self {
                dirty,
                display: false,
                fingerprint: Vec::new(),
                calls: Vec::new(),
                appended: Vec::new(),
            }
        }
    }

    impl Host for FakeHost {
        fn run(&mut self, program: &str, args: &[OsString]) -> Result<()> {
            self.calls.push(format!("{program} {}", join_args(args)));
            Ok(())
        }

        fn output(&mut self, program: &str, args: &[OsString]) -> Result<String> {
            let line = format!("{program} {}", join_args(args));
            self.calls.push(line.clone());
            Ok(match (program, args.first().and_then(|arg| arg.to_str())) {
                ("git", Some("status")) => if self.dirty { " M x\n" } else { "" }.to_owned(),
                ("git", Some("rev-parse")) => "abc123\n".to_owned(),
                ("git", Some("log")) => "2026-11-15T10:00:00+00:00\n".to_owned(),
                ("rustc", _) => "rustc 1.95.0-nightly\n".to_owned(),
                (RUNNER, Some("bench")) => {
                    let scenario = args
                        .get(1)
                        .map_or_else(Default::default, |a| a.to_string_lossy());
                    let print = self.fingerprint.pop().unwrap_or("ab");
                    format!(
                        "INFO started\nbench {scenario}/tick_median 10 ms\nbench \
                         {scenario}/fingerprint {print} hash\n"
                    )
                }
                (VIEWER, Some("--measure")) => {
                    "INFO loading\nbench window/world-60x/frame_median 4.5 ms\n".to_owned()
                }
                (_, Some("bench")) if line.contains("--bench frames") => {
                    "bench viewer/world/frame_median 20 ms\n".to_owned()
                }
                (_, Some("bench")) => {
                    "test geo/distance_to ... bench:  24 ns/iter (+/- 1)\n".to_owned()
                }
                _ => bail!("unexpected call {line}"),
            })
        }

        fn read_to_string(&mut self, path: &Path) -> Result<String> {
            bail!("no {}", path.display())
        }

        fn reset_dir(&mut self, _path: &Path) -> Result<()> {
            bail!("unexpected")
        }

        fn copy(&mut self, _from: &Path, _to: &Path) -> Result<()> {
            bail!("unexpected")
        }

        fn list_dir(&mut self, _path: &Path) -> Result<Vec<String>> {
            bail!("unexpected")
        }

        fn append(&mut self, path: &Path, text: &str) -> Result<()> {
            self.appended.push(format!("{}: {text}", path.display()));
            Ok(())
        }

        fn machine(&mut self, os: &str) -> Machine {
            Machine::from_proc("processor : 0\nmodel name : X\n", "", os)
        }

        fn has_display(&mut self) -> bool {
            self.display
        }
    }

    fn options(groups: &[Group], quick: bool) -> Options {
        Options {
            groups: groups.to_vec(),
            repeat: 3,
            quick,
            cargo: "cargo".to_owned(),
            timestamp: 1_763_200_000,
            os: "linux".to_owned(),
        }
    }

    #[test]
    fn test_full_run_from_a_clean_tree_is_recorded() -> Result<()> {
        let mut host = FakeHost::new(false);
        let outcome = run(&mut host, &options(&[], false))?;
        assert!(outcome.recorded);
        assert_eq!(host.appended.len(), 2);
        assert!(host.appended[0].starts_with("target/bench-runs.jsonl: {\"format\":1"));
        assert!(host.appended[1].starts_with("benchmarks/history.jsonl: {\"format\":1"));
        let record = &outcome.record;
        assert_eq!(record.commit, "abc123");
        assert_eq!(record.machine.cpu, "X");
        assert_eq!(record.groups, ["startup", "world", "micro", "viewer"]);
        for name in [
            "startup/tick_median",
            "tick-06/tick_median",
            "day/fingerprint",
            "micro/geo/distance_to",
            "viewer/world/frame_median",
        ] {
            assert!(record.results.contains_key(name), "{name}");
        }
        // Each start-up and tick scenario three times, the day once.
        let runner_calls = |scenario: &str| {
            host.calls
                .iter()
                .filter(|c| c.starts_with(RUNNER) && c.ends_with(&format!("bench {scenario}")))
                .count()
        };
        assert_eq!(runner_calls("startup"), 3);
        assert_eq!(runner_calls("tick-12"), 3);
        assert_eq!(runner_calls("day"), 1);
        assert!(
            host.calls
                .contains(&"cargo build --release -p aerocosm".to_owned())
        );
        assert!(host.calls.iter().any(|c| c
            == "cargo bench -p aerocosm-core --bench core --profile bench-unwind -- \
                --output-format bencher"));
        Ok(())
    }

    #[test]
    fn test_window_group_runs_where_a_display_exists() -> Result<()> {
        let mut host = FakeHost::new(false);
        host.display = true;
        let outcome = run(&mut host, &options(&[], false))?;
        assert_eq!(
            outcome.record.groups,
            ["startup", "world", "micro", "viewer", "window"]
        );
        assert!(
            outcome
                .record
                .results
                .contains_key("window/world-60x/frame_median")
        );
        assert!(
            host.calls
                .contains(&"cargo build --release -p aerocosm-viewer".to_owned())
        );
        assert!(host.calls.contains(&format!("{VIEWER} --measure")));

        let mut host = FakeHost::new(false);
        host.display = true;
        run(&mut host, &options(&[Group::Window], true))?;
        assert!(host.calls.contains(&format!("{VIEWER} --measure quick")));
        assert!(!host.calls.iter().any(|c| c.contains("-p aerocosm ")));

        // Without a display the default suite leaves it out.
        let mut host = FakeHost::new(false);
        let outcome = run(&mut host, &options(&[], false))?;
        assert!(!outcome.record.groups.contains(&"window".to_owned()));
        assert!(!host.calls.iter().any(|c| c.contains("--measure")));
        Ok(())
    }

    #[test]
    fn test_dirty_or_quick_runs_are_not_recorded() -> Result<()> {
        let mut host = FakeHost::new(true);
        assert!(!run(&mut host, &options(&[Group::Micro], false))?.recorded);
        assert_eq!(host.appended.len(), 1);
        assert!(host.appended[0].starts_with("target/bench-runs.jsonl"));
        // Only the micro group: nothing built for the runner.
        assert!(!host.calls.iter().any(|c| c.contains("build --release")));

        let mut host = FakeHost::new(false);
        let outcome = run(&mut host, &options(&[Group::World, Group::World], true))?;
        assert!(!outcome.recorded);
        assert_eq!(outcome.record.groups, ["world"]);
        assert!(
            host.calls
                .iter()
                .any(|c| c.ends_with("bench tick-00 --settle 2 --ticks 5 --split 2"))
        );
        assert!(
            host.calls
                .iter()
                .any(|c| c.ends_with("bench day --settle 2 --ticks 60 --split 2"))
        );
        Ok(())
    }

    #[test]
    fn test_differing_fingerprints_fail_the_run() {
        let mut host = FakeHost::new(false);
        host.fingerprint = vec!["ab", "cd", "ab"];
        assert!(run(&mut host, &options(&[Group::Startup], false)).is_err());
    }
}
