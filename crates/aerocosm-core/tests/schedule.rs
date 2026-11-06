//! Schedule generation on the shipped data (plan M2, PR 2): calibration,
//! rotations, curfews, feasibility and determinism.

use std::collections::HashMap;

use aerocosm_core::{
    AircraftCatalog, RngSeed, SimDuration, SimTime,
    airline::{AirlineCatalog, AirlineKind},
    schedule::{FlightKind, Schedule, ScheduleConfig, ScheduleInputs, generate},
    time::Weekday,
    traffic::{AirportTarget, ClockTime, TrafficTargets},
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
    Ok(Data {
        world: World::from_json(include_str!("../../../assets/world/germany.world.json"))?,
        aircraft: AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))?,
        airlines: AirlineCatalog::from_json(include_str!(
            "../../../assets/airlines/airlines.json"
        ))?,
        traffic: TrafficTargets::from_json(include_str!("../../../assets/traffic/germany.json"))?,
    })
}

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
    for target in data.traffic.airports() {
        let id = data
            .world
            .airport_by_ident(target.airport())
            .context("target airport")?;
        let wanted: f64 = Weekday::ALL
            .iter()
            .map(|day| target.movements_per_day(*day, factors))
            .sum();
        let got: u32 = schedule.movements(id).iter().sum();
        // Rounding to whole daily slots weighs heavily at small airports.
        let tolerance = wanted * 0.05 + 14.0;
        if (f64::from(got) - wanted).abs() > tolerance {
            bail!(
                "{}: {got} weekly movements, target {wanted:.0}",
                target.airport()
            );
        }
    }
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
    for seed in 0..3 {
        let schedule = schedule(&data, seed)?;
        assert!(schedule.tails().len() > 100);
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
fn test_flights_respect_curfews_in_local_time() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 3)?;
    // Wraps template minutes into the reference week, then reads the airport's
    // local clock.
    let local = |airport, base_minutes: i64, offset: i32| -> Result<Option<ClockTime>> {
        let ident = data.world.airport(airport).context("airport")?.ident();
        let Some(zone) = data.traffic.zone(ident) else {
            return Ok(None);
        };
        let utc = (base_minutes - i64::from(offset)).rem_euclid(WEEK);
        let instant = reference()?.checked_add(SimDuration::from_millis(utc * 60_000))?;
        Ok(Some(ClockTime::from_minute(
            zone.local(instant).minute_of_day(),
        )?))
    };
    let mut checked = 0;
    for (tail_id, tail) in schedule.tails() {
        for flight in schedule.flights_of(tail_id) {
            let events = [
                (flight.origin(), minutes(flight.departure())),
                (flight.destination(), minutes(flight.arrival())),
            ];
            for (airport, minute) in events {
                let ident = data.world.airport(airport).context("airport")?.ident();
                let curfew = data.traffic.airport(ident).and_then(AirportTarget::curfew);
                if let (Some(curfew), Some(time)) =
                    (curfew, local(airport, minute, tail.utc_offset_minutes())?)
                {
                    assert!(!curfew.contains(time), "{ident} movement at {time:?}");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1000);
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
fn test_overflights_match_daily_target() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 5)?;
    let weekly = schedule
        .flights()
        .filter(|(_, flight)| flight.kind() == FlightKind::Overflight)
        .count();
    let per_day = f64::from(u32::try_from(weekly)?) / 7.0;
    let wanted = data.traffic.overflights_per_day();
    assert!(
        (per_day - wanted).abs() < wanted * 0.05,
        "{per_day:.0} vs {wanted:.0}"
    );
    Ok(())
}

#[test]
fn test_cargo_is_flown_by_cargo_airlines_in_freighters() -> Result<()> {
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
        assert_eq!(cargo, freighter);
    }
    assert_eq!(by_kind.len(), 3, "{by_kind:?}");
    Ok(())
}

#[test]
fn test_every_airline_flies_and_few_slots_are_dropped() -> Result<()> {
    let data = data()?;
    let schedule = schedule(&data, 7)?;
    for (index, airline) in data.airlines.iter().enumerate() {
        let tails = schedule.tail_count(aerocosm_core::Id::from_usize(index)?);
        assert!(tails > 0, "{} has no tails", airline.icao());
    }
    assert!(schedule.dropped_slots() * 50 < schedule.flights().len());
    Ok(())
}
