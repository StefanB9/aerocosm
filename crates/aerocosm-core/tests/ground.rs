//! Ground control on the shipped data (plan M4 PR 3): aircraft taxiing at
//! the mapped airports keep apart, never get stuck, and the result is
//! deterministic.

use std::collections::HashMap;

use aerocosm_core::{
    AircraftCatalog, RngSeed, Scenario, SimConfig, SimDuration, SimTime, Simulation,
    airline::AirlineCatalog,
    ground::{GroundDefinition, StandAreasDefinition},
    ops::{FlightView, Phase},
    traffic::TrafficTargets,
    world::World,
};
use anyhow::{Context, Result};

const GROUND: &str = include_str!("../../../assets/ground/germany.ground.json");

fn simulation(seed: u64) -> Result<Simulation> {
    let world = World::from_json(include_str!("../../../assets/world/germany.world.json"))?;
    let scenario = Scenario::with_traffic(
        world,
        AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?,
        AirlineCatalog::from_json(include_str!("../../../assets/airlines/airlines.json"))?,
        TrafficTargets::from_json(include_str!("../../../assets/traffic/germany.json"))?,
    )?
    .with_ground(GroundDefinition::from_json(GROUND)?)?
    .with_stand_areas(StandAreasDefinition::from_json(include_str!(
        "../../../assets/ground/stand-areas.json"
    ))?)?;
    // A summer Wednesday from 06:00 local: the morning departure bank.
    let config = SimConfig::new("2026-07-08T04:00:00Z".parse()?, RngSeed::new(seed));
    Simulation::new(config, scenario).context("cannot set up the simulation")
}

/// The airport whose ground a flight in `phase` is on, if it taxies.
fn taxiing_at(flight: &FlightView) -> Option<usize> {
    match flight.phase() {
        Phase::Pushback | Phase::TaxiOut | Phase::Queue => Some(flight.origin().as_usize()),
        Phase::TaxiIn => Some(flight.destination().as_usize()),
        _ => None,
    }
}

/// The airports with a ground layout, by index.
fn mapped(simulation: &Simulation) -> Result<Vec<bool>> {
    let world = simulation.world();
    let mut mapped = vec![false; world.airport_count()];
    for layout in GroundDefinition::from_json(GROUND)?.airports {
        let id = world
            .airport_by_ident(layout.airport)
            .context("a mapped airport")?;
        mapped[id.as_usize()] = true;
    }
    Ok(mapped)
}

/// What a run shows on the ground of the mapped airports.
#[derive(Debug, Default)]
struct Observed {
    /// Closest two taxiing aircraft at one airport came, m, and who.
    closest: (f64, String),
    /// Longest any aircraft taxied without a break, and who.
    longest: (SimDuration, String),
}

fn describe(flight: &FlightView, simulation: &Simulation) -> String {
    let airport = taxiing_at(flight)
        .and_then(|a| simulation.world().airports().nth(a))
        .map_or_else(String::new, |(_, a)| a.ident().to_string());
    format!(
        "{} tail {} {:?} at {airport} {}",
        flight.callsign(),
        flight.tail().as_usize(),
        flight.phase(),
        flight.position()
    )
}

/// The warm start puts aircraft on the ground without ground control; their
/// legs end within this many ticks.
const SETTLE: u32 = 15 * 60;

/// Runs `simulation` for `hours` after it settles, looking every `every`
/// ticks.
fn observe(simulation: &mut Simulation, hours: u32, every: u32) -> Result<Observed> {
    simulation.run_ticks(SETTLE)?;
    let mapped = mapped(simulation)?;
    let mut observed = Observed {
        closest: (f64::INFINITY, String::new()),
        longest: (SimDuration::ZERO, String::new()),
    };
    let mut since: HashMap<usize, SimTime> = HashMap::new();
    for _ in 0..hours * 3_600 / every {
        simulation.run_ticks(every)?;
        let now = simulation.time();
        let mut by_airport: HashMap<usize, Vec<FlightView>> = HashMap::new();
        let mut still = HashMap::with_capacity(since.len());
        for flight in simulation.flights() {
            let Some(airport) = taxiing_at(&flight).filter(|a| mapped[*a]) else {
                continue;
            };
            let tail = flight.tail().as_usize();
            let start = since.get(&tail).copied().unwrap_or(now);
            still.insert(tail, start);
            let taxied = now.duration_since(start);
            if taxied > observed.longest.0 {
                observed.longest = (taxied, describe(&flight, simulation));
            }
            by_airport.entry(airport).or_default().push(flight);
        }
        since = still;
        for flights in by_airport.values() {
            for (index, a) in flights.iter().enumerate() {
                for b in &flights[index + 1..] {
                    // An escaped aircraft taxies ignoring others, counted as
                    // a ground conflict (D75).
                    if a.ground_conflict() || b.ground_conflict() {
                        continue;
                    }
                    let gap = a.position().distance_to(b.position()).value();
                    if gap < observed.closest.0 {
                        observed.closest = (
                            gap,
                            format!(
                                "{} / {} at {now}",
                                describe(a, simulation),
                                describe(b, simulation)
                            ),
                        );
                    }
                }
            }
        }
    }
    Ok(observed)
}

/// Nobody taxies longer than this: a stand wait ends after 30 minutes
/// (plan M4 D69, D75), and the taxis to the waiting spot and on to the stand
/// take a quarter of an hour each at most.
const LONGEST_TAXI: SimDuration = SimDuration::from_secs(60 * 60);
/// Taxiing aircraft never stand on the same spot (D70) unless one escaped a
/// deadlock (D75): their reference points stay half an airliner's length
/// apart, m.
const CLOSEST: f64 = 20.0;

#[test]
fn test_ground_traffic_keeps_apart_and_flows() -> Result<()> {
    for seed in [3, 7, 11] {
        let mut simulation = simulation(seed)?;
        let observed = observe(&mut simulation, 4, 10)?;
        let stats = simulation.statistics().context("statistics")?.network();
        assert!(stats.taxis() > 500, "seed {seed}: {}", stats.taxis());
        assert!(
            observed.closest.0 >= CLOSEST,
            "seed {seed}: {:.1} m: {}",
            observed.closest.0,
            observed.closest.1
        );
        assert!(
            observed.longest.0 <= LONGEST_TAXI,
            "seed {seed}: {} s: {}",
            observed.longest.0.as_millis() / 1000,
            observed.longest.1
        );
        // Near zero (D75): at most one taxi in a hundred escapes a deadlock
        // in the morning bank.
        assert!(
            stats.ground_conflicts() * 100 <= stats.taxis(),
            "seed {seed}: {} conflicts in {} taxis",
            stats.ground_conflicts(),
            stats.taxis()
        );
        // Stands run short only where few fit the largest freighters (D69).
        assert!(
            stats.stand_shares() * 50 <= stats.taxis(),
            "seed {seed}: {} stand shares in {} taxis",
            stats.stand_shares(),
            stats.taxis()
        );
    }
    Ok(())
}

#[test]
fn test_ground_control_is_deterministic() -> Result<()> {
    let mut a = simulation(3)?;
    let mut b = simulation(3)?;
    a.run_ticks(2 * 3_600)?;
    b.run_ticks(2 * 3_600)?;
    assert!(a == b, "two runs of the same seed differ");
    let taxis = a.statistics().context("statistics")?.network().taxis();
    assert!(taxis > 100, "{taxis}");
    Ok(())
}
