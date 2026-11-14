//! Micro-benchmarks for the hot paths of the core library.
//!
//! Run with `cargo bench -p aerocosm-core --bench core --profile bench-unwind`.
//! `ops/step_*` times one simulation tick of the world (plan M5 D107: under
//! 8 ms).

use std::hint::black_box;

use aerocosm_core::{
    AircraftCatalog, LatLon, RngSeed, Scenario, SimConfig, SimTime, Simulation,
    ground::GroundDefinition,
    traffic::{model::derive, observed::ObservedTraffic},
    units::{Degrees, Meters},
    world::World,
};
use anyhow::Result;
use criterion::{Criterion, criterion_group, criterion_main};

struct Fixture {
    frankfurt: LatLon,
    munich: LatLon,
    east: Degrees,
    hop: Meters,
    start: SimTime,
}

fn fixture() -> Result<Fixture> {
    Ok(Fixture {
        frankfurt: LatLon::new(50.033_333, 8.570_556)?,
        munich: LatLon::new(48.353_783, 11.786_086)?,
        east: Degrees::new(90.0)?,
        hop: Meters::new(250_000.0)?,
        start: "2026-11-05T00:00:00Z".parse()?,
    })
}

fn bench_geo(c: &mut Criterion) {
    let Ok(f) = fixture() else {
        return;
    };
    c.bench_function("geo/distance_to", |b| {
        b.iter(|| black_box(f.frankfurt).distance_to(black_box(f.munich)));
    });
    c.bench_function("geo/initial_bearing_to", |b| {
        b.iter(|| black_box(f.frankfurt).initial_bearing_to(black_box(f.munich)));
    });
    c.bench_function("geo/destination", |b| {
        b.iter(|| black_box(f.frankfurt).destination(black_box(f.east), black_box(f.hop)));
    });
    c.bench_function("geo/interpolate", |b| {
        b.iter(|| black_box(f.frankfurt).interpolate(black_box(f.munich), black_box(0.37)));
    });
}

fn bench_simulation(c: &mut Criterion) {
    let Ok(f) = fixture() else {
        return;
    };
    c.bench_function("sim/run_ticks_1h", |b| {
        b.iter(|| {
            let sim = Simulation::new(SimConfig::new(f.start, RngSeed::new(1)), Scenario::empty());
            black_box(sim.and_then(|mut sim| sim.run_ticks(black_box(3600))))
        });
    });
}

/// The shipped world with its traffic model and ground layouts (plan M5
/// D107: the benchmark measures the real world).
fn world() -> Result<Scenario> {
    let world = World::from_asset(include_bytes!("../../../assets/world/world.bin"))?;
    let aircraft = AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?;
    let observed =
        ObservedTraffic::from_asset(include_bytes!("../../../assets/traffic/observed.bin"))?;
    let (targets, airlines) = derive(&world, &observed, &aircraft)?.into_parts();
    Scenario::with_traffic(world, aircraft, airlines, targets)?.with_ground(
        GroundDefinition::combine(
            [
                &include_bytes!("../../../assets/ground/E.ground.bin")[..],
                &include_bytes!("../../../assets/ground/L.ground.bin")[..],
            ]
            .map(GroundDefinition::from_asset)
            .into_iter()
            .collect::<Result<Vec<_>>>()?,
        )?,
    )
}

fn bench_operations(c: &mut Criterion) {
    let setup = || -> Result<Simulation> {
        let start: SimTime = "2026-07-08T10:00:00Z".parse()?;
        Simulation::new(SimConfig::new(start, RngSeed::new(1)), world()?)
    };
    let Ok(mut sim) = setup() else {
        return;
    };
    let name = format!("ops/step_{}_airborne", sim.airborne());
    c.bench_function(&name, |b| b.iter(|| black_box(sim.step())));
}

criterion_group!(benches, bench_geo, bench_simulation, bench_operations);
criterion_main!(benches);
