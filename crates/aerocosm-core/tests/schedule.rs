//! Schedule generation on the shipped world (plan M2 PR 2, M5 PR 2):
//! calibration, every scheduled airport served, German validation,
//! rotations, feasibility and determinism.

use std::collections::HashMap;

use aerocosm_core::{
    AircraftCatalog, RngSeed, SimDuration, SimTime,
    airline::{AirlineCatalog, AirlineKind},
    schedule::{FlightKind, Schedule, ScheduleConfig, ScheduleInputs, generate},
    time::Weekday,
    traffic::{TrafficTargets, model::derive, observed::ObservedTraffic},
    world::World,
};
use anyhow::{Context, Result, bail};

/// Minutes in the template week.
const WEEK: i64 = 7 * 24 * 60;
/// Shortest turnaround of any type (regional aircraft).
const MIN_TURNAROUND: i64 = 25;

struct Data {
    world: World,
    aircraft: AircraftCatalog,
    airlines: AirlineCatalog,
    traffic: TrafficTargets,
}

fn data() -> Result<Data> {
    let world = World::from_asset(include_bytes!("../../../assets/world/world.bin"))?;
    let aircraft = AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?;
    let observed =
        ObservedTraffic::from_asset(include_bytes!("../../../assets/traffic/observed.bin"))?;
    let (traffic, airlines) = derive(&world, &observed, &aircraft)?.into_parts();
    Ok(Data {
        world,
        aircraft,
        airlines,
        traffic,
    })
}

/// ADV-Monatsstatistik 12/2019: scheduled and charter movements in 2019
/// (Berlin: Tegel and Schönefeld). Validation only (plan M5 D105, D114).
const ADV_2019: [(&str, f64); 11] = [
    ("EDDF", 502_634.0),
    ("EDDM", 395_951.0),
    ("EDDB", 269_127.0),
    ("EDDL", 214_595.0),
    ("EDDH", 136_959.0),
    ("EDDK", 128_855.0),
    ("EDDS", 117_011.0),
    ("EDDP", 70_044.0),
    ("EDDV", 55_178.0),
    ("EDDN", 39_460.0),
    ("EDDW", 21_640.0),
];

/// Monday 6 July 2026, 00:00 UTC: a summer reference week.
fn reference() -> Result<SimTime> {
    "2026-07-06T00:00:00Z".parse()
}

fn schedule(data: &Data, seed: u64) -> Result<Schedule> {
    let inputs = ScheduleInputs::new(&data.world, &data.aircraft, &data.airlines, &data.traffic)?;
    generate(
        &inputs,
        ScheduleConfig::new(RngSeed::new(seed), reference()?),
    )
}

fn minutes(duration: SimDuration) -> i64 {
    duration.as_millis() / 60_000
}

#[test]
fn test_generate_is_deterministic() -> Result<()> {
    let data = data()?;
    let first = schedule(&data, 11)?;
    assert_eq!(first, schedule(&data, 11)?);
    assert_ne!(first, schedule(&data, 12)?);
    Ok(())
}

#[test]
fn test_generate_matches_weekly_movement_targets() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 1)?;
    let factors = data.traffic.weekday_factors();
    let (mut off, mut count, mut wanted_total, mut got_total) = (Vec::new(), 0, 0.0, 0.0);
    for target in data.traffic.airports() {
        let id = data
            .world
            .airport_by_ident(target.airport())
            .context("target airport")?;
        let wanted: f64 = Weekday::ALL
            .iter()
            .map(|day| target.movements_per_day(*day, factors))
            .sum();
        let got = f64::from(schedule.movements(id).iter().sum::<u32>());
        count += 1;
        wanted_total += wanted;
        got_total += got;
        // Many small services, each rounded to whole flights a day, add up
        // to a count that varies like a Poisson count: three standard
        // deviations, and 5 % for the rest.
        if (got - wanted).abs() > wanted * 0.05 + 3.0 * wanted.sqrt() + 7.0 {
            off.push(format!("{} {got:.0}/{wanted:.0}", target.airport()));
        }
    }
    // Airports whose slots or runways cannot take their target fall short
    // (docs/known-issues.md): few, and the world total holds.
    assert!(off.len() * 50 < count, "{} off: {off:?}", off.len());
    assert!(
        (got_total / wanted_total - 1.0).abs() < 0.02,
        "{got_total:.0} of {wanted_total:.0}"
    );
    Ok(())
}

#[test]
fn test_generate_movements_follow_weekday_factors() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 2)?;
    let frankfurt = data
        .world
        .airport_by_ident("EDDF".parse()?)
        .context("EDDF")?;
    let days = schedule.movements(frankfurt);
    let factors = data.traffic.weekday_factors();
    let (low, high) = (0..7).fold((0, 0), |(low, high), day| {
        (
            if factors[day] < factors[low] {
                day
            } else {
                low
            },
            if factors[day] > factors[high] {
                day
            } else {
                high
            },
        )
    });
    assert!(days[high] > days[low], "{days:?}");
    Ok(())
}

#[test]
fn test_rotations_are_continuous_over_the_week() -> Result<()> {
    let data = data()?;
    for seed in 0..1 {
        let schedule = schedule(&data, seed)?;
        assert!(schedule.tails().len() > 10_000);
        for (tail_id, tail) in schedule.tails() {
            let flights = schedule.flights_of(tail_id);
            let (Some(first), Some(last)) = (flights.first(), flights.last()) else {
                bail!("tail {} has no flights", tail_id.index());
            };
            assert_eq!(first.origin(), tail.base());
            assert_eq!(last.destination(), tail.base());
            assert!(minutes(first.departure()) < WEEK);
            for pair in flights.windows(2) {
                assert_eq!(pair[0].destination(), pair[1].origin());
                assert!(
                    minutes(pair[1].departure()) >= minutes(pair[0].arrival()) + MIN_TURNAROUND,
                    "tail {} turns too fast",
                    tail_id.index()
                );
            }
            assert!(
                minutes(last.arrival()) + MIN_TURNAROUND <= minutes(first.departure()) + WEEK,
                "tail {} cannot repeat its week",
                tail_id.index()
            );
            for flight in flights {
                assert_eq!(flight.tail(), tail_id);
                assert_eq!(
                    (flight.airline(), flight.aircraft()),
                    (tail.airline(), tail.aircraft())
                );
            }
        }
    }
    Ok(())
}

#[test]
fn test_scheduled_airports_are_served() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 3)?;
    let mut unserved = Vec::new();
    for target in data.traffic.airports() {
        let id = data
            .world
            .airport_by_ident(target.airport())
            .context("target airport")?;
        if schedule.movements(id).iter().sum::<u32>() == 0 {
            unserved.push((
                target.airport().to_string(),
                f64::from(target.movements_per_year()) / 365.0,
            ));
        }
    }
    // A week can pass without a flight where a few a week are scheduled.
    let busy: Vec<_> = unserved.iter().filter(|(_, daily)| *daily >= 4.0).collect();
    assert!(busy.is_empty(), "{busy:?}");
    assert!(
        unserved.len() * 50 < data.traffic.airports().count(),
        "{} unserved",
        unserved.len()
    );
    // About 105,000 departures a day (plan M5).
    let per_day = f64::from(u32::try_from(schedule.flights().count())?) / 7.0;
    assert!((95_000.0..115_000.0).contains(&per_day), "{per_day:.0}");
    Ok(())
}

#[test]
fn test_aircraft_can_fly_every_sector() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 4)?;
    for (_, flight) in schedule.flights() {
        let aircraft = data.aircraft.get(flight.aircraft()).context("aircraft")?;
        let origin = data.world.airport(flight.origin()).context("origin")?;
        let destination = data
            .world
            .airport(flight.destination())
            .context("destination")?;
        let distance = origin.location().distance_to(destination.location());
        assert!(aircraft.range().to_meters().value() >= distance.value());
        for airport in [flight.origin(), flight.destination()] {
            let longest = data
                .world
                .runways_of(airport)
                .iter()
                .map(|runway| runway.length().value())
                .fold(0.0, f64::max);
            assert!(longest >= aircraft.min_runway_length().value() * 0.75);
        }
        assert!(flight.block() > SimDuration::from_minutes(30));
    }
    Ok(())
}

#[test]
fn test_german_movements_match_adv_2019() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 5)?;
    let mut failures = Vec::new();
    for (ident, yearly) in ADV_2019 {
        let id = data
            .world
            .airport_by_ident(ident.parse()?)
            .context("airport")?;
        let weekly: u32 = schedule.movements(id).iter().sum();
        let ratio = f64::from(weekly) * 365.0 / 7.0 / yearly;
        if !(0.75..=1.25).contains(&ratio) {
            failures.push(format!("{ident} {ratio:.2}"));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
    Ok(())
}

#[test]
fn test_cargo_is_flown_by_cargo_airlines() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 6)?;
    let mut by_kind: HashMap<FlightKind, usize> = HashMap::new();
    for (_, flight) in schedule.flights() {
        *by_kind.entry(flight.kind()).or_default() += 1;
        let airline = data.airlines.get(flight.airline()).context("airline")?;
        let freighter = data
            .aircraft
            .get(flight.aircraft())
            .context("aircraft")?
            .is_freighter();
        let cargo = flight.kind() == FlightKind::Cargo;
        assert_eq!(cargo, airline.kind() == AirlineKind::Cargo);
        // Freighter-only types carry no passengers; cargo airlines may fly
        // converted passenger types.
        assert!(cargo || !freighter, "{} in a freighter", airline.icao());
    }
    assert_eq!(by_kind.len(), 2, "{by_kind:?}");
    Ok(())
}

#[test]
fn test_every_airline_flies_and_few_slots_are_dropped() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 7)?;
    let mut idle = Vec::new();
    for (index, airline) in data.airlines.iter().enumerate() {
        if schedule.tail_count(aerocosm_core::Id::from_usize(index)?) == 0 {
            idle.push(airline.icao().to_string());
        }
    }
    assert!(idle.len() * 50 < data.airlines.len(), "{idle:?}");
    assert!(schedule.dropped_slots() * 50 < schedule.flights().len());
    Ok(())
}
