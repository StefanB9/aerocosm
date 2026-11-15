//! The world scenarios (plan M5 D135, D138): start-up, ticks at fixed hours
//! of a Wednesday in July, and a simulated day.

use std::{fmt, str::FromStr, time::Duration};

use aerocosm_core::{
    AircraftCatalog, RngSeed, Scenario, SimConfig, SimTime, Simulation, WallClock,
    ground::GroundDefinition,
    ops::PhaseTimes,
    traffic::{model::derive, observed::ObservedTraffic},
    world::World,
};
use anyhow::{Context, Result, bail};

use super::{Measure, Unit, Value, memory, stats::Summary};

/// The day the scenarios run on: a Wednesday in July, schedules at their
/// fullest.
const DAY: &str = "2026-07-08";

/// Ticks in a day.
pub(crate) const DAY_TICKS: u32 = 86_400;

/// A benchmark scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScenarioName {
    /// Decoding the inputs and setting up the simulation, warm start at
    /// noon.
    Startup,
    /// Ticks from the given hour (UTC).
    Tick(u8),
    /// A whole day from midnight.
    Day,
}

impl ScenarioName {
    /// When the simulation starts.
    fn start(self) -> Result<SimTime> {
        let hour = match self {
            Self::Startup => 12,
            Self::Tick(hour) => hour,
            Self::Day => 0,
        };
        format!("{DAY}T{hour:02}:00:00Z").parse()
    }
}

impl fmt::Display for ScenarioName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup => f.write_str("startup"),
            Self::Tick(hour) => write!(f, "tick-{hour:02}"),
            Self::Day => f.write_str("day"),
        }
    }
}

impl FromStr for ScenarioName {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "startup" => Ok(Self::Startup),
            "day" => Ok(Self::Day),
            _ => {
                let hour = text
                    .strip_prefix("tick-")
                    .filter(|hour| hour.len() == 2)
                    .and_then(|hour| hour.parse::<u8>().ok())
                    .filter(|hour| *hour < 24);
                match hour {
                    Some(hour) => Ok(Self::Tick(hour)),
                    None => bail!("unknown scenario {text} (startup, tick-HH or day)"),
                }
            }
        }
    }
}

/// The scenario inputs, as file contents.
pub(crate) struct Inputs {
    pub(crate) world: Vec<u8>,
    pub(crate) aircraft: Vec<u8>,
    pub(crate) observed: Vec<u8>,
    /// Each ground region's name (its letter) and contents.
    pub(crate) regions: Vec<(String, Vec<u8>)>,
}

/// How long the scenarios run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lengths {
    /// Ticks run before measuring.
    pub(crate) settle: u32,
    /// Ticks measured one by one; for the day, the ticks of the run.
    pub(crate) ticks: u32,
    /// Ticks timed by part after those.
    pub(crate) split: u32,
}

/// Times `work` with `clock`.
fn timed<T>(clock: &impl WallClock, work: impl FnOnce() -> Result<T>) -> Result<(T, Duration)> {
    let start = clock.now();
    let value = work()?;
    Ok((value, clock.now().saturating_since(start)))
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// Memory now, as measures named `{prefix}_peak` and `{prefix}_resident`.
fn memory_measures(prefix: &str, out: &mut Vec<Measure>) {
    if let Some(memory) = memory::current() {
        out.push(Measure::number(
            format!("{prefix}_peak"),
            memory.peak,
            Unit::MiB,
        ));
        out.push(Measure::number(
            format!("{prefix}_resident"),
            memory.resident,
            Unit::MiB,
        ));
    }
}

/// Decodes the inputs and sets up the simulation from `start`, timing each
/// step into `out` (plan M5 D138).
fn set_up(
    inputs: &Inputs,
    config: SimConfig,
    clock: &impl WallClock,
    out: &mut Vec<Measure>,
) -> Result<Simulation> {
    let mut total = Duration::ZERO;
    let mut step = |name: String, spent: Duration, out: &mut Vec<Measure>| {
        total += spent;
        out.push(Measure::number(name, ms(spent), Unit::Milliseconds));
    };
    let (world, spent) = timed(clock, || World::from_asset(&inputs.world).context("world"))?;
    step("startup/decode/world".into(), spent, out);
    let (aircraft, spent) = timed(clock, || {
        let text = std::str::from_utf8(&inputs.aircraft).context("aircraft catalog")?;
        AircraftCatalog::from_json(text).context("aircraft catalog")
    })?;
    step("startup/decode/aircraft".into(), spent, out);
    let (observed, spent) = timed(clock, || {
        ObservedTraffic::from_asset(&inputs.observed).context("observed traffic")
    })?;
    step("startup/decode/observed".into(), spent, out);
    let mut regions = Vec::with_capacity(inputs.regions.len());
    for (name, bytes) in &inputs.regions {
        let (region, spent) = timed(clock, || {
            GroundDefinition::from_asset(bytes).with_context(|| format!("ground region {name}"))
        })?;
        regions.push(region);
        step(format!("startup/decode/ground/{name}"), spent, out);
    }
    let (scenario, spent) = timed(clock, || {
        let (targets, airlines) = derive(&world, &observed, &aircraft)
            .context("world model")?
            .into_parts();
        Scenario::with_traffic(world, aircraft, airlines, targets)
    })?;
    step("startup/derive".into(), spent, out);
    let (scenario, spent) = timed(clock, || {
        if regions.is_empty() {
            return Ok(scenario);
        }
        scenario.with_ground(GroundDefinition::combine(regions)?)
    })?;
    step("startup/ground_networks".into(), spent, out);
    let (simulation, spent) = timed(clock, || Simulation::new(config, scenario))?;
    step("startup/simulation".into(), spent, out);
    out.push(Measure::number(
        "startup/total",
        ms(total),
        Unit::Milliseconds,
    ));
    Ok(simulation)
}

/// Runs scenario `name` on `inputs` with `seed`, timed by `clock`.
pub(crate) fn run(
    name: ScenarioName,
    inputs: &Inputs,
    seed: RngSeed,
    lengths: Lengths,
    clock: &impl WallClock,
) -> Result<Vec<Measure>> {
    let config = SimConfig::new(name.start()?, seed);
    let mut startup = Vec::new();
    let mut simulation = set_up(inputs, config, clock, &mut startup)?;
    let mut out = Vec::new();
    match name {
        ScenarioName::Startup => {
            out = startup;
            memory_measures("startup/memory", &mut out);
        }
        ScenarioName::Tick(_) => ticks(&mut simulation, name, lengths, clock, &mut out)?,
        ScenarioName::Day => day(&mut simulation, lengths.ticks, clock, &mut out)?,
    }
    Ok(out)
}

/// Ticks measured one by one, then by part.
fn ticks(
    simulation: &mut Simulation,
    name: ScenarioName,
    lengths: Lengths,
    clock: &impl WallClock,
    out: &mut Vec<Measure>,
) -> Result<()> {
    simulation.run_ticks(lengths.settle).context("settling")?;
    let mut samples = Vec::with_capacity(lengths.ticks as usize);
    for _ in 0..lengths.ticks {
        let ((), spent) = timed(clock, || simulation.step())?;
        samples.push(ms(spent));
    }
    if let Some(summary) = Summary::of(&mut samples) {
        for (figure, value) in [
            ("median", summary.median),
            ("p95", summary.p95),
            ("max", summary.max),
            ("mean", summary.mean),
        ] {
            out.push(Measure::number(
                format!("{name}/tick_{figure}"),
                value,
                Unit::Milliseconds,
            ));
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let count = |n: usize| n as f64;
    out.push(Measure::number(
        format!("{name}/airborne"),
        count(simulation.airborne()),
        Unit::Count,
    ));
    out.push(Measure::number(
        format!("{name}/taxiing"),
        count(simulation.flights().filter(|f| f.taxi().is_some()).count()),
        Unit::Count,
    ));
    let mut parts = PhaseTimes::default();
    let ((), split_wall) = timed(clock, || {
        for _ in 0..lengths.split {
            parts += simulation.step_timed(clock)?;
        }
        Ok(())
    })?;
    if lengths.split > 0 {
        let per_tick = |time: Duration| ms(time) / f64::from(lengths.split);
        // The timed ticks' own mean, to set the parts against: they are
        // later ticks than the measured ones, and timing costs a little.
        out.push(Measure::number(
            format!("{name}/part/tick_mean"),
            per_tick(split_wall),
            Unit::Milliseconds,
        ));
        for (part, time) in [
            ("flow_control", parts.flow_control),
            ("ground_control", parts.ground_control),
            ("taxiing", parts.taxiing),
            ("flying", parts.flying),
            ("parked", parts.parked),
        ] {
            out.push(Measure::number(
                format!("{name}/part/{part}"),
                per_tick(time),
                Unit::Milliseconds,
            ));
        }
    }
    out.push(fingerprint(&name.to_string(), simulation));
    memory_measures(&format!("{name}/memory"), out);
    Ok(())
}

/// `ticks` ticks from midnight in one go.
fn day(
    simulation: &mut Simulation,
    ticks: u32,
    clock: &impl WallClock,
    out: &mut Vec<Measure>,
) -> Result<()> {
    let start = memory::current();
    let ((), spent) = timed(clock, || simulation.run_ticks(ticks))?;
    out.push(Measure::number(
        "day/wall",
        spent.as_secs_f64(),
        Unit::Seconds,
    ));
    if !spent.is_zero() {
        out.push(Measure::number(
            "day/ticks_per_second",
            f64::from(ticks) / spent.as_secs_f64(),
            Unit::PerSecond,
        ));
    }
    #[allow(clippy::cast_precision_loss)]
    out.push(Measure::number(
        "day/flights",
        simulation.flights_flown() as f64,
        Unit::Count,
    ));
    out.push(fingerprint("day", simulation));
    if let Some(start) = start {
        out.push(Measure::number(
            "day/memory_start",
            start.resident,
            Unit::MiB,
        ));
    }
    memory_measures("day/memory", out);
    Ok(())
}

fn fingerprint(scenario: &str, simulation: &Simulation) -> Measure {
    Measure {
        name: format!("{scenario}/fingerprint"),
        value: Value::Hash(simulation.fingerprint()),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::run::SystemClock;

    fn inputs(regions: &[(&str, &[u8])]) -> Inputs {
        Inputs {
            world: include_bytes!("../../../../assets/world/world.bin").to_vec(),
            aircraft: include_bytes!("../../../../assets/aircraft/types.json").to_vec(),
            observed: include_bytes!("../../../../assets/traffic/observed.bin").to_vec(),
            regions: regions
                .iter()
                .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
                .collect(),
        }
    }

    fn names(measures: &[Measure]) -> Vec<&str> {
        measures.iter().map(|m| m.name.as_str()).collect()
    }

    fn number(measures: &[Measure], name: &str) -> Result<f64> {
        match measures.iter().find(|m| m.name == name).map(|m| m.value) {
            Some(Value::Number(value, _)) => Ok(value),
            other => anyhow::bail!("{name}: {other:?}"),
        }
    }

    const SHORT: Lengths = Lengths {
        settle: 2,
        ticks: 5,
        split: 2,
    };

    #[test]
    fn test_scenario_names_round_trip() -> Result<()> {
        for name in ["startup", "tick-00", "tick-06", "tick-12", "tick-23", "day"] {
            assert_eq!(name.parse::<ScenarioName>()?.to_string(), name);
        }
        for bad in ["tick-24", "tick-6", "tick-xx", "noon", ""] {
            assert!(bad.parse::<ScenarioName>().is_err(), "{bad}");
        }
        assert_eq!(
            ScenarioName::Tick(6).start()?,
            "2026-07-08T06:00:00Z".parse()?
        );
        Ok(())
    }

    #[test]
    fn test_startup_times_each_input_and_step() -> Result<()> {
        let region = include_bytes!("../../../../assets/ground/E.ground.bin");
        let out = run(
            ScenarioName::Startup,
            &inputs(&[("E", region)]),
            RngSeed::new(3),
            SHORT,
            &SystemClock::new(),
        )?;
        let names = names(&out);
        for name in [
            "startup/decode/world",
            "startup/decode/aircraft",
            "startup/decode/observed",
            "startup/decode/ground/E",
            "startup/derive",
            "startup/ground_networks",
            "startup/simulation",
            "startup/total",
        ] {
            assert!(names.contains(&name), "{name}: {names:?}");
        }
        let parts: f64 = out
            .iter()
            .filter(|m| m.name != "startup/total" && !m.name.contains("memory"))
            .filter_map(|m| match m.value {
                Value::Number(value, Unit::Milliseconds) => Some(value),
                _ => None,
            })
            .sum();
        assert!((number(&out, "startup/total")? - parts).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn test_tick_scenario_reports_ticks_parts_and_fingerprint() -> Result<()> {
        let out = run(
            ScenarioName::Tick(12),
            &inputs(&[]),
            RngSeed::new(3),
            SHORT,
            &SystemClock::new(),
        )?;
        let names = names(&out);
        for name in [
            "tick-12/tick_median",
            "tick-12/tick_p95",
            "tick-12/tick_max",
            "tick-12/airborne",
            "tick-12/taxiing",
            "tick-12/part/ground_control",
            "tick-12/part/flying",
            "tick-12/fingerprint",
        ] {
            assert!(names.contains(&name), "{name}: {names:?}");
        }
        assert!(number(&out, "tick-12/airborne")? > 1_000.0);
        assert!(!names.iter().any(|n| n.starts_with("startup/")));
        // The same run gives the same fingerprint.
        let again = run(
            ScenarioName::Tick(12),
            &inputs(&[]),
            RngSeed::new(3),
            SHORT,
            &SystemClock::new(),
        )?;
        let print = |m: &[Measure]| m.iter().find(|m| m.name.ends_with("fingerprint")).cloned();
        assert_eq!(print(&out), print(&again));
        assert!(print(&out).is_some());
        Ok(())
    }

    #[test]
    fn test_day_scenario_reports_throughput() -> Result<()> {
        let out = run(
            ScenarioName::Day,
            &inputs(&[]),
            RngSeed::new(3),
            Lengths { ticks: 10, ..SHORT },
            &SystemClock::new(),
        )?;
        let names = names(&out);
        for name in ["day/wall", "day/flights", "day/fingerprint"] {
            assert!(names.contains(&name), "{name}: {names:?}");
        }
        assert!(number(&out, "day/wall")? > 0.0);
        let fingerprint = out
            .iter()
            .find(|m| m.name == "day/fingerprint")
            .context("fingerprint")?;
        assert!(matches!(fingerprint.value, Value::Hash(_)));
        Ok(())
    }
}
