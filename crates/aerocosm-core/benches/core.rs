//! Micro-benchmarks for the hot paths of the core library.
//!
//! Run with `cargo bench -p aerocosm-core --bench core --profile bench-unwind`.
//! `ops/step_*` times one simulation tick with German traffic scaled up until
//! about 15,000 aircraft are airborne: the plan's target is under 1 ms.

use std::hint::black_box;

use aerocosm_core::{
    AircraftCatalog, LatLon, RngSeed, Scenario, SimConfig, SimTime, Simulation,
    airline::AirlineCatalog,
    ground::{GroundDefinition, StandAreasDefinition},
    traffic::TrafficTargets,
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

/// How much the German movement and overflight targets are scaled up.
const TRAFFIC_SCALE: f64 = 23.0;

/// The shipped scenario, ground layouts included, with every traffic target
/// multiplied by `scale`.
fn scaled_germany(scale: f64) -> Result<Scenario> {
    let mut traffic =
        TrafficTargets::from_json(include_str!("../../../assets/traffic/germany.json"))?
            .to_definition();
    for airport in &mut traffic.airports {
        // Scaled yearly movements stay far below u32::MAX.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (f64::from(airport.movements_per_year) * scale).round() as u32;
        airport.movements_per_year = scaled;
        // Coordinated capacities scale with the traffic, or slot coordination
        // would spread the extra flights over the whole day.
        if let Some(capacity) = airport.capacity.as_mut() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let grow = |value: u16| (f64::from(value) * scale).min(f64::from(u16::MAX)) as u16;
            capacity.per_hour = grow(capacity.per_hour);
            capacity.per_window = grow(capacity.per_window);
        }
    }
    traffic.overflights_per_day *= scale;
    Scenario::with_traffic(
        World::from_json(include_str!("../../../assets/world/germany.world.json"))?,
        AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?,
        AirlineCatalog::from_json(include_str!("../../../assets/airlines/airlines.json"))?,
        TrafficTargets::from_definition(traffic)?,
    )?
    .with_ground(GroundDefinition::from_json(include_str!(
        "../../../assets/ground/germany.ground.json"
    ))?)?
    .with_stand_areas(StandAreasDefinition::from_json(include_str!(
        "../../../assets/ground/stand-areas.json"
    ))?)
}

fn bench_operations(c: &mut Criterion) {
    let setup = || -> Result<Simulation> {
        let start: SimTime = "2026-07-08T10:00:00Z".parse()?;
        Simulation::new(
            SimConfig::new(start, RngSeed::new(1)),
            scaled_germany(TRAFFIC_SCALE)?,
        )
    };
    let Ok(mut sim) = setup() else {
        return;
    };
    let name = format!("ops/step_{}_airborne", sim.airborne());
    c.bench_function(&name, |b| b.iter(|| black_box(sim.step())));
}

criterion_group!(benches, bench_geo, bench_simulation, bench_operations);
criterion_main!(benches);
