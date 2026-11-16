//! Micro-benchmarks for the hot paths of the core library (plan M5 D139).
//!
//! Run with `cargo bench -p aerocosm-core --bench core --profile bench-unwind`,
//! or as part of `cargo xtask bench micro`. `geo/*` times great-circle maths,
//! `sim/*` the empty world's ticks, `ground/*` the lookups ground control
//! makes at Frankfurt. The world at scale, its schedule included, is
//! measured by `aerocosm bench` (D135, D138): the German world alone has too
//! few observed airports to fit the traffic model.

use std::{hint::black_box, io::Write as _, process::ExitCode};

use aerocosm_core::{
    Id, LatLon, RngSeed, Scenario, SimConfig, SimTime, Simulation,
    ground::{AirportGround, GroundDefinition, GroundNetwork, Node, StandRequest},
    units::{Degrees, Meters},
    world::World,
};
use anyhow::{Context, Result};
use criterion::Criterion;

fn bench_geo(c: &mut Criterion) -> Result<()> {
    let frankfurt = LatLon::new(50.033_333, 8.570_556)?;
    let munich = LatLon::new(48.353_783, 11.786_086)?;
    let east = Degrees::new(90.0)?;
    let hop = Meters::new(250_000.0)?;
    c.bench_function("geo/distance_to", |b| {
        b.iter(|| black_box(frankfurt).distance_to(black_box(munich)));
    });
    c.bench_function("geo/initial_bearing_to", |b| {
        b.iter(|| black_box(frankfurt).initial_bearing_to(black_box(munich)));
    });
    c.bench_function("geo/destination", |b| {
        b.iter(|| black_box(frankfurt).destination(black_box(east), black_box(hop)));
    });
    c.bench_function("geo/interpolate", |b| {
        b.iter(|| black_box(frankfurt).interpolate(black_box(munich), black_box(0.37)));
    });
    Ok(())
}

fn bench_simulation(c: &mut Criterion) -> Result<()> {
    let start: SimTime = "2026-11-05T00:00:00Z".parse()?;
    c.bench_function("sim/run_ticks_1h", |b| {
        b.iter(|| {
            let sim = Simulation::new(SimConfig::new(start, RngSeed::new(1)), Scenario::empty());
            black_box(sim.and_then(|mut sim| sim.run_ticks(black_box(3600))))
        });
    });
    Ok(())
}

/// The first holding position's node.
fn first_hold(ground: &AirportGround) -> Result<Id<Node>> {
    ground
        .holding_positions()
        .first()
        .map(aerocosm_core::ground::HoldingPosition::node)
        .context("no holding position")
}

/// Frankfurt's ground: a taxi route across the airport and a stand choice.
fn bench_ground(c: &mut Criterion) -> Result<()> {
    let world = World::from_asset(include_bytes!("../../../assets/world/world.bin"))?;
    let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
    let near = world.airport(frankfurt).context("EDDF")?.location();
    let region =
        GroundDefinition::from_asset(include_bytes!("../../../assets/ground/E.ground.bin"))?;
    let network = GroundNetwork::from_definition(region, &world)?;
    let ground = network.airport(frankfurt).context("EDDF's ground")?;
    let wingspan = Meters::new(65.0)?;
    // From the stand farthest from the first holding position: a long route.
    let hold = first_hold(ground)?;
    let at = |node: Id<Node>| ground.nodes()[node.as_usize()].position();
    let distance = |node: Id<Node>| at(node).distance_to(at(hold)).value();
    let stand = ground
        .stands()
        .iter()
        .map(aerocosm_core::ground::Stand::node)
        .filter(|node| ground.route(*node, hold, wingspan).is_some())
        .max_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .context("no stand with a route to the holding position")?;
    c.bench_function("ground/route_frankfurt", |b| {
        b.iter(|| black_box(ground.route(black_box(stand), black_box(hold), wingspan)));
    });
    let request = StandRequest {
        wingspan,
        near,
        cargo: false,
    };
    c.bench_function("ground/choose_stand_frankfurt", |b| {
        b.iter(|| {
            black_box(ground.choose_stand(
                black_box(&request),
                |s| s.as_usize() % 7 == 0,
                |s| s.as_usize() % 3 != 0,
            ))
        });
    });
    Ok(())
}

/// A group of benchmarks, failing if it cannot set up.
type Group = fn(&mut Criterion) -> Result<()>;

/// Runs every group; a group that cannot set up fails the run, so that a
/// benchmark never disappears from the history unnoticed.
fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    let groups: [(&str, Group); 3] = [
        ("geo", bench_geo),
        ("sim", bench_simulation),
        ("ground", bench_ground),
    ];
    let mut failed = false;
    for (name, group) in groups {
        if let Err(error) = group(&mut criterion) {
            let _ = writeln!(std::io::stderr(), "benchmark group {name}: {error:#}");
            failed = true;
        }
    }
    criterion.final_summary();
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
