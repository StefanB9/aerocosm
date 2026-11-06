//! Schedule generation: a deterministic weekly template of flights grouped into
//! tails (plan M2, PR 2).

use std::fmt;

use anyhow::{Context, Result};

use crate::{
    AircraftCatalog, AircraftType, Id, RngSeed, SimDuration, SimTime,
    airline::{Airline, AirlineCatalog},
    traffic::TrafficTargets,
    world::{Airport, World},
};

mod demand;
mod generate;
mod network;
mod overflight;
mod rotation;
mod timetable;
mod trips;

pub use generate::generate;
pub(crate) use timetable::min_turnaround;

/// What a scheduled flight carries and where it flies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightKind {
    /// A passenger flight to or from a German airport.
    Passenger,
    /// A cargo flight to or from a German airport.
    Cargo,
    /// A flight between two edge airports that crosses the simulated airspace.
    Overflight,
}

/// An airline's flight number, e.g. `400` in `LH400` / `DLH400`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlightNumber(u16);

impl FlightNumber {
    /// Wraps a flight number.
    pub const fn new(number: u16) -> Self {
        Self(number)
    }

    /// The number.
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl fmt::Display for FlightNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The data a schedule is generated from, with cross-references checked.
#[derive(Debug, Clone, Copy)]
pub struct ScheduleInputs<'a> {
    world: &'a World,
    aircraft: &'a AircraftCatalog,
    airlines: &'a AirlineCatalog,
    traffic: &'a TrafficTargets,
}

impl<'a> ScheduleInputs<'a> {
    /// Bundles the inputs after checking that every airline base and traffic
    /// target airport exists in `world` and every fleet type in `aircraft`.
    pub fn new(
        world: &'a World,
        aircraft: &'a AircraftCatalog,
        airlines: &'a AirlineCatalog,
        traffic: &'a TrafficTargets,
    ) -> Result<Self> {
        airlines
            .check_references(world, aircraft)
            .context("airline catalog does not match the world")?;
        traffic
            .check_references(world)
            .context("traffic targets do not match the world")?;
        Ok(Self {
            world,
            aircraft,
            airlines,
            traffic,
        })
    }
}

/// How a schedule is generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleConfig {
    seed: RngSeed,
    reference: SimTime,
}

impl ScheduleConfig {
    /// Draws from `seed`'s scheduling stream and takes zone offsets at
    /// `reference` (D43).
    pub const fn new(seed: RngSeed, reference: SimTime) -> Self {
        Self { seed, reference }
    }

    /// The run's seed.
    pub const fn seed(self) -> RngSeed {
        self.seed
    }

    /// The instant whose zone offsets place base-local times at other airports.
    pub const fn reference(self) -> SimTime {
        self.reference
    }
}

/// One aircraft: it flies a fixed rotation from its base every week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tail {
    airline: Id<Airline>,
    aircraft: Id<AircraftType>,
    base: Id<Airport>,
    utc_offset_minutes: i32,
}

impl Tail {
    /// The operating airline.
    pub const fn airline(&self) -> Id<Airline> {
        self.airline
    }

    /// The aircraft type.
    pub const fn aircraft(&self) -> Id<AircraftType> {
        self.aircraft
    }

    /// Where the rotation starts and ends each week.
    pub const fn base(&self) -> Id<Airport> {
        self.base
    }

    /// The base's UTC offset at the reference instant, in minutes: subtract it
    /// from a base-local template time to get UTC.
    pub const fn utc_offset_minutes(&self) -> i32 {
        self.utc_offset_minutes
    }
}

/// One flight of the weekly template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledFlight {
    airline: Id<Airline>,
    number: FlightNumber,
    aircraft: Id<AircraftType>,
    tail: Id<Tail>,
    origin: Id<Airport>,
    destination: Id<Airport>,
    departure: SimDuration,
    block: SimDuration,
    kind: FlightKind,
}

impl ScheduledFlight {
    /// The operating airline.
    pub const fn airline(&self) -> Id<Airline> {
        self.airline
    }

    /// The flight number; the callsign is the airline's ICAO code plus this.
    pub const fn number(&self) -> FlightNumber {
        self.number
    }

    /// The aircraft type.
    pub const fn aircraft(&self) -> Id<AircraftType> {
        self.aircraft
    }

    /// The aircraft flying it.
    pub const fn tail(&self) -> Id<Tail> {
        self.tail
    }

    /// Departure airport.
    pub const fn origin(&self) -> Id<Airport> {
        self.origin
    }

    /// Arrival airport.
    pub const fn destination(&self) -> Id<Airport> {
        self.destination
    }

    /// Scheduled departure since Monday 00:00 in the tail's base-local time
    /// (D43). Within a rotation departures only increase, so the return leg
    /// of a trip that leaves late on Sunday may lie past one week: take it
    /// modulo a week to place it.
    pub const fn departure(&self) -> SimDuration {
        self.departure
    }

    /// Block time, gate to gate.
    pub const fn block(&self) -> SimDuration {
        self.block
    }

    /// Scheduled arrival on the same scale as [`Self::departure`].
    pub const fn arrival(&self) -> SimDuration {
        SimDuration::from_millis(self.departure.as_millis() + self.block.as_millis())
    }

    /// Passenger, cargo or overflight.
    pub const fn kind(&self) -> FlightKind {
        self.kind
    }
}

/// A weekly schedule: tails and their flights, repeated every week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    tails: Vec<Tail>,
    /// Grouped by tail, each tail's flights in departure order.
    flights: Vec<ScheduledFlight>,
    /// Start of each tail's flights in `flights`, plus a final end marker.
    tail_starts: Vec<usize>,
    /// Movements per airport (by index) and local weekday, Monday first.
    movements: Vec<[u32; 7]>,
    dropped_slots: usize,
}

impl Schedule {
    /// All tails.
    pub fn tails(&self) -> impl ExactSizeIterator<Item = (Id<Tail>, &Tail)> {
        self.tails
            .iter()
            .enumerate()
            .map(|(index, tail)| (Id::new(u32::try_from(index).unwrap_or(u32::MAX)), tail))
    }

    /// One tail.
    pub fn tail(&self, id: Id<Tail>) -> Option<&Tail> {
        self.tails.get(id.as_usize())
    }

    /// All flights, grouped by tail.
    pub fn flights(
        &self,
    ) -> impl ExactSizeIterator<Item = (Id<ScheduledFlight>, &ScheduledFlight)> {
        self.flights
            .iter()
            .enumerate()
            .map(|(index, flight)| (Id::new(u32::try_from(index).unwrap_or(u32::MAX)), flight))
    }

    /// A tail's weekly rotation in departure order; empty for an unknown tail.
    pub fn flights_of(&self, tail: Id<Tail>) -> &[ScheduledFlight] {
        let index = tail.as_usize();
        match (self.tail_starts.get(index), self.tail_starts.get(index + 1)) {
            (Some(start), Some(end)) => &self.flights[*start..*end],
            _ => &[],
        }
    }

    /// Departures plus arrivals at `airport` per local weekday, Monday first.
    pub fn movements(&self, airport: Id<Airport>) -> [u32; 7] {
        self.movements
            .get(airport.as_usize())
            .copied()
            .unwrap_or_default()
    }

    /// Number of tails the airline needs (D38).
    pub fn tail_count(&self, airline: Id<Airline>) -> usize {
        self.tails
            .iter()
            .filter(|tail| tail.airline == airline)
            .count()
    }

    /// Slot-days that found no feasible aircraft or curfew-free times.
    pub const fn dropped_slots(&self) -> usize {
        self.dropped_slots
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::test_data::data;

    fn flight(departure: i32, block: i32) -> ScheduledFlight {
        ScheduledFlight {
            airline: Id::new(0),
            number: FlightNumber::new(400),
            aircraft: Id::new(1),
            tail: Id::new(0),
            origin: Id::new(2),
            destination: Id::new(3),
            departure: SimDuration::from_minutes(departure),
            block: SimDuration::from_minutes(block),
            kind: FlightKind::Passenger,
        }
    }

    fn tiny() -> Schedule {
        Schedule {
            tails: vec![Tail {
                airline: Id::new(0),
                aircraft: Id::new(1),
                base: Id::new(2),
                utc_offset_minutes: 60,
            }],
            flights: vec![flight(600, 70), flight(700, 70)],
            tail_starts: vec![0, 2],
            movements: vec![[0; 7], [0; 7], [1, 0, 0, 0, 0, 0, 1]],
            dropped_slots: 3,
        }
    }

    #[test]
    fn test_flight_number_display() {
        assert_eq!(FlightNumber::new(400).to_string(), "400");
        assert_eq!(FlightNumber::new(7).value(), 7);
    }

    #[test]
    fn test_scheduled_flight_arrival_adds_block() {
        let flight = flight(10_070, 95);
        assert_eq!(flight.arrival(), SimDuration::from_minutes(10_165));
        assert_eq!(flight.number(), FlightNumber::new(400));
        assert_eq!(
            (flight.origin(), flight.destination()),
            (Id::new(2), Id::new(3))
        );
    }

    #[test]
    fn test_schedule_accessors() {
        let schedule = tiny();
        assert_eq!(schedule.tails().len(), 1);
        assert_eq!(schedule.flights().len(), 2);
        assert_eq!(schedule.flights_of(Id::new(0)).len(), 2);
        assert_eq!(schedule.flights_of(Id::new(1)).len(), 0);
        assert_eq!(
            schedule.tail(Id::new(0)).map(Tail::utc_offset_minutes),
            Some(60)
        );
        assert_eq!(schedule.movements(Id::new(2)), [1, 0, 0, 0, 0, 0, 1]);
        assert_eq!(schedule.movements(Id::new(9)), [0; 7]);
        assert_eq!(
            (
                schedule.tail_count(Id::new(0)),
                schedule.tail_count(Id::new(1))
            ),
            (1, 0)
        );
        assert_eq!(schedule.dropped_slots(), 3);
    }

    #[test]
    fn test_schedule_config_accessors() -> Result<()> {
        let reference: SimTime = "2026-07-06T00:00:00Z".parse()?;
        let config = ScheduleConfig::new(RngSeed::new(5), reference);
        assert_eq!(
            (config.seed(), config.reference()),
            (RngSeed::new(5), reference)
        );
        Ok(())
    }

    #[test]
    fn test_schedule_inputs_check_references() -> Result<()> {
        let data = data()?;
        assert!(
            ScheduleInputs::new(&data.world, &data.aircraft, &data.airlines, &data.traffic).is_ok()
        );
        let empty = World::empty();
        assert!(
            ScheduleInputs::new(&empty, &data.aircraft, &data.airlines, &data.traffic).is_err()
        );
        Ok(())
    }
}
