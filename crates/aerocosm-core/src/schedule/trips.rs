//! From services to the week's round trips: aircraft type, daily slots, times
//! and curfews.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::{
    FlightKind, FlightNumber,
    network::Service,
    timetable::{
        Profile, block_time, min_turnaround, offset_minutes, pick_weighted, sample_minute,
        stochastic_round,
    },
};
use crate::{
    AircraftCatalog, AircraftType, Id, SimRng, SimTime, WakeCategory,
    airline::{Airline, AirlineCatalog, Network},
    time::Weekday,
    traffic::{ClockTime, Curfew, TrafficTargets},
    units::Meters,
    world::{Airport, World},
};

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
/// Passenger wide-bodies only fly sectors at least this long…
const WIDE_BODY_MIN_METERS: f64 = 1_200_000.0;
/// …and regional aircraft at most this long, unless nothing else fits.
const REGIONAL_MAX_METERS: f64 = 1_500_000.0;
/// Point-to-point departures from this base-local time may stay overnight at
/// the far end.
const NIGHTSTOP_FROM: u32 = 19 * 60;
/// Probability that a late departure becomes a nightstop.
const NIGHTSTOP_CHANCE: f64 = 0.5;
/// Nightstop returns leave between 06:00 and 07:30 far-end local time.
const MORNING_RETURN: u32 = 6 * 60;
const MORNING_SPREAD: u32 = 90;
/// Hub flights leave this many minutes after a bank, plus up to
/// [`BANK_SPREAD`].
const BANK_OFFSET: u32 = 30;
const BANK_SPREAD: u32 = 30;
/// Extra ground time beyond the minimum turnaround, up to this many minutes.
const GROUND_SPREAD: u32 = 30;
/// Each airline's flight numbers start here.
const FIRST_FLIGHT_NUMBER: u16 = 100;

/// Lookups the trip builder needs, with zone offsets taken at a reference
/// instant (D43).
pub(crate) struct TripContext<'a> {
    world: &'a World,
    aircraft: &'a AircraftCatalog,
    airlines: &'a AirlineCatalog,
    weekday_factors: [f64; 7],
    offsets: HashMap<Id<Airport>, i64>,
    curfews: HashMap<Id<Airport>, Curfew>,
    banks: HashMap<Id<Airport>, Vec<ClockTime>>,
}

impl<'a> TripContext<'a> {
    /// Prepares offsets, curfews and banks for every airport with a known zone.
    pub(crate) fn new(
        world: &'a World,
        aircraft: &'a AircraftCatalog,
        airlines: &'a AirlineCatalog,
        traffic: &TrafficTargets,
        reference: SimTime,
    ) -> Result<Self> {
        let mut offsets = HashMap::new();
        for (id, airport) in world.airports() {
            if let Some(zone) = traffic.zone(airport.ident()) {
                offsets.insert(id, i64::from(offset_minutes(zone, reference)));
            }
        }
        let mut curfews = HashMap::new();
        let mut banks = HashMap::new();
        for target in traffic.airports() {
            let id = world
                .airport_by_ident(target.airport())
                .with_context(|| format!("target airport {} not in the world", target.airport()))?;
            if let Some(curfew) = target.curfew() {
                curfews.insert(id, curfew);
            }
            if !target.banks().is_empty() {
                banks.insert(id, target.banks().to_vec());
            }
        }
        Ok(Self {
            world,
            aircraft,
            airlines,
            weekday_factors: *traffic.weekday_factors(),
            offsets,
            curfews,
            banks,
        })
    }

    fn airport(&self, id: Id<Airport>) -> Result<&'a Airport> {
        self.world.airport(id).context("airport id out of range")
    }

    /// Great-circle distance between two airports.
    pub(crate) fn distance(&self, a: Id<Airport>, b: Id<Airport>) -> Result<Meters> {
        Ok(self
            .airport(a)?
            .location()
            .distance_to(self.airport(b)?.location()))
    }

    /// Length of the airport's longest runway; zero without runways.
    pub(crate) fn longest_runway(&self, airport: Id<Airport>) -> Meters {
        self.world
            .runways_of(airport)
            .iter()
            .map(crate::world::Runway::length)
            .fold(Meters::default(), |a, b| if b > a { b } else { a })
    }

    /// The airport's night curfew, if any.
    pub(crate) fn curfew(&self, airport: Id<Airport>) -> Option<Curfew> {
        self.curfews.get(&airport).copied()
    }

    fn offset(&self, airport: Id<Airport>) -> i64 {
        self.offsets.get(&airport).copied().unwrap_or(0)
    }

    /// Converts a base-local week minute to local time of day at `airport`.
    pub(crate) fn local_minute(
        &self,
        base: Id<Airport>,
        airport: Id<Airport>,
        minute: u32,
    ) -> ClockTime {
        let local = i64::from(minute) - self.offset(base) + self.offset(airport);
        // SAFETY: rem_euclid(1440) is in 0..1440, which fits u16 and is a valid
        // clock time.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minute_of_day = local.rem_euclid(i64::from(DAY)) as u16;
        ClockTime::from_minute(minute_of_day).unwrap_or(ClockTime::MIDNIGHT)
    }
}

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

/// One aircraft type per service, drawn from the airline's fleet mix among
/// types that can fly the sector; size rules (no short-haul wide-bodies, no
/// long-haul regionals) are relaxed only if nothing else fits.
fn choose_aircraft(
    ctx: &TripContext<'_>,
    airline: &Airline,
    service: &Service,
    distance: Meters,
    rng: &mut SimRng,
) -> Option<Id<AircraftType>> {
    let shortest_runway = ctx
        .longest_runway(service.base)
        .value()
        .min(ctx.longest_runway(service.other).value());
    let options: Vec<(Id<AircraftType>, &AircraftType, f64)> = airline
        .fleet()
        .iter()
        .filter_map(|share| {
            let id = ctx.aircraft.by_code(share.aircraft)?;
            let aircraft = ctx.aircraft.get(id)?;
            let flies = aircraft.range().to_meters().value() >= distance.value() * RANGE_MARGIN
                && shortest_runway >= aircraft.min_runway_length().value() * RUNWAY_MARGIN;
            flies.then_some((id, aircraft, share.share))
        })
        .collect();
    let sized = |aircraft: &AircraftType| {
        let wide = !aircraft.is_freighter()
            && matches!(aircraft.wake(), WakeCategory::Heavy | WakeCategory::Super);
        let regional = !aircraft.is_freighter() && aircraft.seats() <= 100;
        let short_wide = wide && distance.value() < WIDE_BODY_MIN_METERS;
        let long_regional = regional && distance.value() > REGIONAL_MAX_METERS;
        !short_wide && !long_regional
    };
    let weights: Vec<f64> = options
        .iter()
        .map(|(_, aircraft, share)| if sized(aircraft) { *share } else { 0.0 })
        .collect();
    let index = pick_weighted(rng, &weights).or_else(|| {
        let shares: Vec<f64> = options.iter().map(|(_, _, share)| *share).collect();
        pick_weighted(rng, &shares)
    })?;
    Some(options[index].0)
}

/// The departure time from the base for a slot, before curfew checks.
fn departure(ctx: &TripContext<'_>, airline: &Airline, service: &Service, rng: &mut SimRng) -> u32 {
    if service.kind == FlightKind::Cargo {
        return u32::from(sample_minute(rng, Profile::CargoNight));
    }
    if airline.network() == Network::Hub
        && let Some(banks) = ctx.banks.get(&service.base)
        && let Ok(index) = usize::try_from(
            rng.below(u32::try_from(banks.len()).unwrap_or(1))
                .unwrap_or(0),
        )
        && let Some(bank) = banks.get(index)
    {
        return (u32::from(bank.minute_of_day())
            + BANK_OFFSET
            + rng.below(BANK_SPREAD).unwrap_or(0))
            % DAY;
    }
    u32::from(sample_minute(rng, Profile::PassengerDay))
}

/// Ground time at the far end: a turnaround, or for some late point-to-point
/// departures a nightstop returning next morning far-end local time (D42).
fn ground_time(
    ctx: &TripContext<'_>,
    airline: &Airline,
    service: &Service,
    depart: u32,
    out_block: u32,
    turnaround: u32,
    rng: &mut SimRng,
) -> u32 {
    let ground = turnaround + rng.below(GROUND_SPREAD + 1).unwrap_or(0);
    let nightstop = service.kind == FlightKind::Passenger
        && airline.network() != Network::Hub
        && depart >= NIGHTSTOP_FROM
        && rng.next_f64() < NIGHTSTOP_CHANCE;
    if !nightstop {
        return ground;
    }
    // Arrival in far-end local minutes on an absolute scale, then the next
    // 06:00-07:30.
    let arrive =
        i64::from(depart + out_block) - ctx.offset(service.base) + ctx.offset(service.other);
    let wanted = i64::from(MORNING_RETURN + rng.below(MORNING_SPREAD).unwrap_or(0));
    let mut morning = arrive.div_euclid(i64::from(DAY)) * i64::from(DAY) + wanted;
    while morning < arrive + i64::from(turnaround) {
        morning += i64::from(DAY);
    }
    u32::try_from(morning - arrive).unwrap_or(ground)
}

/// Builds every service's round trips for the week template, drawing from
/// `rng`.
pub(crate) fn build_trips(
    ctx: &TripContext<'_>,
    services: &[Service],
    rng: &mut SimRng,
) -> Result<TripsOutcome> {
    let mut trips = Vec::with_capacity(services.len() * 10);
    let mut dropped = 0;
    let mut next_number: HashMap<Id<Airline>, u16> = HashMap::new();
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
        let block = minutes(block_time(distance, aircraft));
        let turnaround = minutes(min_turnaround(aircraft));
        for slot in 0..slots {
            let days = counts.iter().filter(|count| **count > slot).count();
            let placed = (0..PLACEMENT_ATTEMPTS).find_map(|_| {
                let depart = departure(ctx, airline, service, rng);
                let ground = ground_time(ctx, airline, service, depart, block, turnaround, rng);
                let trip = RoundTrip {
                    airline: service.airline,
                    base: service.base,
                    other: service.other,
                    aircraft: aircraft_id,
                    numbers: (FlightNumber::new(0), FlightNumber::new(0)),
                    start: depart,
                    out_block: block,
                    ground,
                    back_block: block,
                    base_turnaround: turnaround,
                    kind: service.kind,
                };
                trip.events()
                    .iter()
                    .all(|(airport, minute)| {
                        ctx.curfew(*airport).is_none_or(|c| {
                            !c.contains(ctx.local_minute(service.base, *airport, *minute))
                        })
                    })
                    .then_some(trip)
            });
            let Some(template) = placed else {
                dropped += days;
                continue;
            };
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
            network::{plan_services, tests::data},
            timetable::Profile,
        },
    };

    fn context(data: &super::super::network::tests::Data) -> Result<TripContext<'_>> {
        TripContext::new(
            &data.world,
            &data.aircraft,
            &data.airlines,
            &data.traffic,
            "2026-11-05T12:00:00Z".parse()?,
        )
    }

    fn build(seed: u64) -> Result<(super::super::network::tests::Data, TripsOutcome)> {
        let data = data()?;
        let services = plan_services(&data.world, &data.airlines, &data.traffic)?;
        let mut rng = RngSeed::new(seed).stream(StreamId::Scheduling);
        let outcome = build_trips(&context(&data)?, &services, &mut rng)?;
        Ok((data, outcome))
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
    fn test_cargo_departs_at_night() -> Result<()> {
        let (_, outcome) = build(4)?;
        let night = Profile::CargoNight.weights();
        let cargo: Vec<&RoundTrip> = outcome
            .trips
            .iter()
            .filter(|t| t.kind == FlightKind::Cargo)
            .collect();
        assert_ne!(cargo.len(), 0);
        for trip in cargo {
            let hour = usize::try_from(trip.start % DAY / 60).context("hour")?;
            assert!(night[hour] > 0.0, "cargo departure at hour {hour}");
        }
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
}
