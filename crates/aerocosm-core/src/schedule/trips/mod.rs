//! From services to the week's round trips: aircraft type, daily slots, times
//! and curfews.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::{
    FlightKind, FlightNumber,
    network::Service,
    timetable::{block_time, min_turnaround, stochastic_round},
};
use crate::{AircraftType, Id, SimRng, airline::Airline, time::Weekday, world::Airport};

mod choice;
mod context;

use self::choice::{choose_aircraft, departure, ground_time};
pub(crate) use self::context::TripContext;

/// Minutes in a day.
pub(crate) const DAY: u32 = 24 * 60;
/// An aircraft must have this much more range than the sector length.
pub(crate) const RANGE_MARGIN: f64 = 1.1;
/// Short sectors fly below maximum take-off weight, so runways down to this
/// fraction of the published take-off field length are accepted.
pub(crate) const RUNWAY_MARGIN: f64 = 0.75;
/// Tries to find departure times that respect every curfew before a slot is
/// dropped.
const PLACEMENT_ATTEMPTS: u32 = 20;
/// Slot coordination shifts a drawn time in steps of this many minutes…
const SHIFT_STEP: i64 = 5;
/// …up to this far either way (D54).
const MAX_SHIFT: i64 = 60;

/// Offsets tried for a drawn time: 0, +5, -5, +10, -10, … minutes.
fn shifts() -> impl Iterator<Item = i64> {
    std::iter::once(0).chain(
        (1..=MAX_SHIFT / SHIFT_STEP).flat_map(|step| [step * SHIFT_STEP, -step * SHIFT_STEP]),
    )
}

/// Each airline's flight numbers start here.
const FIRST_FLIGHT_NUMBER: u16 = 100;

/// One round trip of the week template, in base-local week minutes (D43).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoundTrip {
    pub(crate) airline: Id<Airline>,
    pub(crate) base: Id<Airport>,
    pub(crate) other: Id<Airport>,
    pub(crate) aircraft: Id<AircraftType>,
    /// Outbound and return flight numbers.
    pub(crate) numbers: (FlightNumber, FlightNumber),
    /// Departure from the base, minutes since Monday 00:00 base-local.
    pub(crate) start: u32,
    pub(crate) out_block: u32,
    /// Time at the far end, including a nightstop.
    pub(crate) ground: u32,
    pub(crate) back_block: u32,
    /// Minimum turnaround at the base before the tail's next trip.
    pub(crate) base_turnaround: u32,
    pub(crate) kind: FlightKind,
}

impl RoundTrip {
    /// Arrival back at the base.
    pub(crate) const fn end(&self) -> u32 {
        self.start + self.out_block + self.ground + self.back_block
    }

    /// The four movements: departure, far-end arrival, far-end departure,
    /// arrival back.
    pub(crate) const fn events(&self) -> [(Id<Airport>, u32); 4] {
        let arrive = self.start + self.out_block;
        [
            (self.base, self.start),
            (self.other, arrive),
            (self.other, arrive + self.ground),
            (self.base, self.end()),
        ]
    }
}

/// The week's round trips and how many slot-days could not be placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TripsOutcome {
    pub(crate) trips: Vec<RoundTrip>,
    pub(crate) dropped: usize,
}

fn minutes(duration: crate::SimDuration) -> u32 {
    u32::try_from(duration.as_millis() / 60_000).unwrap_or(0)
}

/// At most this many trips are reserved up front.
const MAX_RESERVED_TRIPS: usize = 1 << 22;

/// About how many round trips `services` fly in a week: the sum of their
/// daily frequencies. The world has some ten million services that hardly
/// ever fly, so reserving per service would ask for gigabytes (more than
/// wasm32 can address).
fn expected_trips(services: &[Service]) -> usize {
    let week: f64 = services
        .iter()
        .map(|service| service.daily.max(0.0))
        .sum::<f64>()
        * 7.0;
    if !week.is_finite() {
        return 0;
    }
    // SAFETY: a capacity hint of a finite, non-negative sum; a huge one
    // saturates and is capped.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let trips = week.ceil() as usize;
    trips.min(MAX_RESERVED_TRIPS)
}

/// Builds every service's round trips for the week template, drawing from
/// `rng`.
pub(crate) fn build_trips(
    ctx: &TripContext<'_>,
    services: &[Service],
    rng: &mut SimRng,
) -> Result<TripsOutcome> {
    let mut trips = Vec::with_capacity(expected_trips(services));
    let mut dropped = 0;
    let mut next_number: HashMap<Id<Airline>, u16> = HashMap::new();
    let mut books = ctx.slot_books();
    for service in services {
        let airline = ctx
            .airlines
            .get(service.airline)
            .context("airline id out of range")?;
        let counts: Vec<u32> = Weekday::ALL
            .iter()
            .map(|day| stochastic_round(rng, service.daily * ctx.weekday_factors[day.index()]))
            .collect();
        let slots = counts.iter().copied().max().unwrap_or(0);
        if slots == 0 {
            continue;
        }
        let distance = ctx.distance(service.base, service.other)?;
        let Some(aircraft_id) = choose_aircraft(ctx, airline, service, distance, rng) else {
            dropped += counts.iter().map(|count| *count as usize).sum::<usize>();
            continue;
        };
        let aircraft = ctx
            .aircraft
            .get(aircraft_id)
            .context("aircraft id out of range")?;
        let out_block = minutes(block_time(
            distance,
            aircraft,
            ctx.taxi(service.base, service.other),
        ));
        let back_block = minutes(block_time(
            distance,
            aircraft,
            ctx.taxi(service.other, service.base),
        ));
        let turnaround = minutes(min_turnaround(aircraft, service.kind == FlightKind::Cargo));
        for slot in 0..slots {
            let days: Vec<u32> = (0_u32..)
                .zip(&counts)
                .filter(|(_, count)| **count > slot)
                .map(|(day, _)| day)
                .collect();
            let placed = (0..PLACEMENT_ATTEMPTS).find_map(|attempt| {
                let depart = departure(ctx, airline, service, attempt, rng);
                let ground = ground_time(ctx, airline, service, depart, out_block, turnaround, rng);
                // Shift the drawn time before giving up on it (D54).
                shifts().find_map(|shift| {
                    let start = (i64::from(depart) + shift).rem_euclid(i64::from(DAY));
                    let trip = RoundTrip {
                        airline: service.airline,
                        base: service.base,
                        other: service.other,
                        aircraft: aircraft_id,
                        numbers: (FlightNumber::new(0), FlightNumber::new(0)),
                        start: u32::try_from(start).ok()?,
                        out_block,
                        ground,
                        back_block,
                        base_turnaround: turnaround,
                        kind: service.kind,
                    };
                    (ctx.curfew_free(&trip) && ctx.coordinated(&books, &trip, &days))
                        .then_some(trip)
                })
            });
            let Some(template) = placed else {
                dropped += days.len();
                continue;
            };
            ctx.book(&mut books, &template, &days);
            let number = next_number
                .entry(service.airline)
                .or_insert(FIRST_FLIGHT_NUMBER);
            let numbers = (
                FlightNumber::new(*number),
                FlightNumber::new(number.wrapping_add(1)),
            );
            *number = number.wrapping_add(2);
            for (day, count) in counts.iter().enumerate() {
                if *count > slot {
                    let day = u32::try_from(day).context("weekday index")?;
                    trips.push(RoundTrip {
                        numbers,
                        start: day * DAY + template.start,
                        ..template
                    });
                }
            }
        }
    }
    Ok(TripsOutcome { trips, dropped })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed, StreamId,
        schedule::{
            network::plan_services,
            rotation::WEEK,
            timetable::{Profile, TaxiMinutes},
        },
        test_data::{Data, data},
    };

    fn context(data: &Data) -> Result<TripContext<'_>> {
        TripContext::new(
            &data.world,
            &data.aircraft,
            &data.airlines,
            &data.traffic,
            "2026-11-05T12:00:00Z".parse()?,
        )
    }

    #[test]
    fn test_context_taxi_minutes_default_or_mapped() -> Result<()> {
        let data = data()?;
        let frankfurt = data
            .world
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let munich = data
            .world
            .airport_by_ident("EDDM".parse()?)
            .context("EDDM")?;
        let plain = context(&data)?;
        assert_eq!(plain.taxi(frankfurt, munich), TaxiMinutes::DEFAULT);
        let mut table = vec![None; data.world.airport_count()];
        table[frankfurt.as_usize()] = Some(crate::ground::TaxiTimes {
            out: crate::SimDuration::from_secs(600),
            back: crate::SimDuration::from_secs(420),
        });
        let mapped = context(&data)?.with_taxi(&table);
        let taxi = mapped.taxi(frankfurt, munich);
        assert!((taxi.out - 10.0).abs() < 1e-9);
        assert!((taxi.back - TaxiMinutes::DEFAULT.back).abs() < 1e-9);
        let back = mapped.taxi(munich, frankfurt);
        assert!((back.out - TaxiMinutes::DEFAULT.out).abs() < 1e-9);
        assert!((back.back - 7.0).abs() < 1e-9);
        Ok(())
    }

    fn build(seed: u64) -> Result<(Data, TripsOutcome)> {
        let data = data()?;
        let services = plan_services(&data.world, &data.aircraft, &data.airlines, &data.traffic)?;
        let mut rng = RngSeed::new(seed).stream(StreamId::Scheduling);
        let outcome = build_trips(&context(&data)?, &services, &mut rng)?;
        Ok((data, outcome))
    }

    #[test]
    fn test_expected_trips_follow_frequencies_not_services() {
        let service = |daily: f64| Service {
            airline: Id::new(0),
            base: Id::new(0),
            other: Id::new(1),
            daily,
            kind: FlightKind::Passenger,
        };
        // A world's worth of services that hardly ever fly reserve little:
        // ten trips each would be 4.4 GB, more than wasm32 can address.
        let rare = vec![service(1e-4); 1_000_000];
        assert!(expected_trips(&rare) <= 1_000, "{}", expected_trips(&rare));
        // A week of a twice-daily service.
        assert_eq!(expected_trips(&[service(2.0)]), 14);
        assert_eq!(expected_trips(&[]), 0);
        assert_eq!(expected_trips(&[service(f64::INFINITY)]), 0);
        assert_eq!(expected_trips(&[service(1e12)]), MAX_RESERVED_TRIPS);
    }

    #[test]
    fn test_trips_are_deterministic() -> Result<()> {
        let (_, a) = build(7)?;
        let (_, b) = build(7)?;
        assert_eq!(a.trips, b.trips);
        let (_, c) = build(8)?;
        assert_ne!(a.trips, c.trips);
        Ok(())
    }

    #[test]
    fn test_trips_use_feasible_aircraft() -> Result<()> {
        let (data, outcome) = build(1)?;
        let ctx = context(&data)?;
        for trip in &outcome.trips {
            let aircraft = data.aircraft.get(trip.aircraft).context("type")?;
            let distance = ctx.distance(trip.base, trip.other)?;
            assert!(
                aircraft.range().to_meters().value() >= distance.value() * RANGE_MARGIN,
                "{} too far",
                aircraft.code()
            );
            for airport in [trip.base, trip.other] {
                assert!(
                    ctx.longest_runway(airport).value()
                        >= aircraft.min_runway_length().value() * RUNWAY_MARGIN
                );
            }
        }
        Ok(())
    }

    #[test]
    fn test_trips_respect_curfews_at_both_ends() -> Result<()> {
        let (data, outcome) = build(2)?;
        let ctx = context(&data)?;
        for trip in &outcome.trips {
            for (airport, base_minute) in trip.events() {
                let local = ctx.local_minute(trip.base, airport, base_minute);
                if let Some(curfew) = ctx.curfew(airport) {
                    assert!(
                        !curfew.contains(local),
                        "event at {local:?} during curfew at {airport:?}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_slots_keep_number_and_time_across_days() -> Result<()> {
        let (_, outcome) = build(3)?;
        let mut by_number: HashMap<(Id<Airline>, FlightNumber), HashSet<u32>> = HashMap::new();
        for trip in &outcome.trips {
            by_number
                .entry((trip.airline, trip.numbers.0))
                .or_default()
                .insert(trip.start % DAY);
        }
        assert!(
            by_number.values().all(|times| times.len() == 1),
            "a flight number keeps its time of day"
        );
        assert!(by_number.len() > 1000, "{} slots", by_number.len());
        Ok(())
    }

    #[test]
    fn test_cargo_departs_at_night_where_nothing_crowds_it() -> Result<()> {
        let (data, outcome) = build(4)?;
        let ctx = context(&data)?;
        let night = Profile::CargoNight.weights();
        let cargo: Vec<&RoundTrip> = outcome
            .trips
            .iter()
            .filter(|t| t.kind == FlightKind::Cargo)
            // Where a curfew or slots crowd the night, cargo moves to the day.
            .filter(|t| ctx.curfew(t.base).is_none() && ctx.curfew(t.other).is_none())
            .filter(|t| {
                !ctx.capacities.contains_key(&t.base) && !ctx.capacities.contains_key(&t.other)
            })
            .collect();
        assert_ne!(cargo.len(), 0);
        for trip in cargo {
            let hour = usize::try_from(trip.start % DAY / 60).context("hour")?;
            assert!(night[hour] > 0.0, "cargo departure at hour {hour}");
        }
        Ok(())
    }

    #[test]
    fn test_cargo_at_coordinated_airports_moves_to_the_day() -> Result<()> {
        let data = data()?;
        let frankfurt = data
            .world
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let touches = |kind: FlightKind, base: Id<Airport>, other: Id<Airport>| {
            kind == FlightKind::Cargo && (base == frankfurt || other == frankfurt)
        };
        let services = plan_services(&data.world, &data.aircraft, &data.airlines, &data.traffic)?;
        let wanted: f64 = services
            .iter()
            .filter(|s| touches(s.kind, s.base, s.other))
            .map(|s| s.daily * 7.0)
            .sum();
        let mut rng = RngSeed::new(9).stream(StreamId::Scheduling);
        let outcome = build_trips(&context(&data)?, &services, &mut rng)?;
        let placed = outcome
            .trips
            .iter()
            .filter(|t| touches(t.kind, t.base, t.other))
            .count();
        // Frankfurt's slots are full at night and at the peaks: most cargo
        // still finds a time.
        assert!(wanted > 10.0, "{wanted}");
        assert!(
            f64::from(u32::try_from(placed)?) >= wanted * 0.8,
            "{placed} of {wanted:.0} placed"
        );
        Ok(())
    }

    #[test]
    fn test_few_slots_are_dropped() -> Result<()> {
        let (_, outcome) = build(5)?;
        let placed = outcome.trips.len();
        assert!(
            outcome.dropped * 50 < placed,
            "{} dropped of {placed}",
            outcome.dropped
        );
        Ok(())
    }

    #[test]
    fn test_coordinated_airports_stay_within_capacity() -> Result<()> {
        let (data, outcome) = build(6)?;
        let ctx = context(&data)?;
        let mut checked = 0;
        for target in data.traffic.airports() {
            let Some(capacity) = target.capacity() else {
                continue;
            };
            let airport = data
                .world
                .airport_by_ident(target.airport())
                .context("airport")?;
            let minutes: Vec<u32> = outcome
                .trips
                .iter()
                .flat_map(|trip| {
                    trip.events()
                        .into_iter()
                        .map(move |event| (trip.base, event))
                })
                .filter(|(_, (at, _))| *at == airport)
                .map(|(base, (_, minute))| ctx.local_week_minute(base, airport, minute))
                .collect();
            let window = u32::from(capacity.window_minutes);
            for start in (0..WEEK).step_by(usize::try_from(window)?) {
                let since = |m: &&u32| (**m + WEEK - start) % WEEK;
                let in_window = minutes.iter().filter(|m| since(m) < window).count();
                let in_hour = minutes.iter().filter(|m| since(m) < 60).count();
                assert!(
                    in_window <= usize::from(capacity.per_window),
                    "{}: {in_window} in the window from minute {start}",
                    target.airport()
                );
                assert!(
                    in_hour <= usize::from(capacity.per_hour),
                    "{}: {in_hour} in the hour from minute {start}",
                    target.airport()
                );
            }
            checked += 1;
        }
        assert!(checked >= 5);
        Ok(())
    }
}
