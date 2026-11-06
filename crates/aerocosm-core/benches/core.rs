//! Micro-benchmarks for the hot paths of the core library.
//!
//! Run with `cargo bench -p aerocosm-core`. The M2 milestone adds the
//! 15k-aircraft step benchmark that guards the global-scale performance target.

use std::hint::black_box;

use aerocosm_core::{
    LatLon, RngSeed, Scenario, SimConfig, SimTime, Simulation,
    units::{Degrees, Meters},
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

criterion_group!(benches, bench_geo, bench_simulation);
criterion_main!(benches);
