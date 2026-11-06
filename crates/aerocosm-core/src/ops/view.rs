//! Read-only views of active flights.

use std::fmt;

use super::Phase;
use crate::{
    AircraftType, Id, LatLon, SimTime,
    airline::{Airline, AirlineIcao},
    schedule::{FlightKind, FlightNumber, Tail},
    units::{Degrees, Feet, Knots},
    world::Airport,
};

/// Radio callsign: airline ICAO code and flight number, e.g. `DLH400`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Callsign {
    airline: AirlineIcao,
    number: FlightNumber,
}

impl Callsign {
    /// The callsign of `airline`'s flight `number`.
    pub const fn new(airline: AirlineIcao, number: FlightNumber) -> Self {
        Self { airline, number }
    }

    /// The airline's ICAO code.
    pub const fn airline(self) -> AirlineIcao {
        self.airline
    }

    /// The flight number.
    pub const fn number(self) -> FlightNumber {
        self.number
    }
}

impl fmt::Display for Callsign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.airline, self.number)
    }
}

/// A snapshot of one active flight: boarding, moving or just landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlightView {
    pub(super) tail: Id<Tail>,
    pub(super) callsign: Callsign,
    pub(super) airline: Id<Airline>,
    pub(super) aircraft: Id<AircraftType>,
    pub(super) kind: FlightKind,
    pub(super) origin: Id<Airport>,
    pub(super) destination: Id<Airport>,
    pub(super) phase: Phase,
    pub(super) position: LatLon,
    pub(super) heading: Degrees,
    pub(super) altitude: Feet,
    pub(super) ground_speed: Knots,
    pub(super) scheduled_departure: SimTime,
    pub(super) scheduled_arrival: SimTime,
    pub(super) off_block: SimTime,
}

impl FlightView {
    /// The aircraft flying it.
    pub const fn tail(&self) -> Id<Tail> {
        self.tail
    }

    /// Airline ICAO code and flight number.
    pub const fn callsign(&self) -> Callsign {
        self.callsign
    }

    /// The operating airline.
    pub const fn airline(&self) -> Id<Airline> {
        self.airline
    }

    /// The aircraft type.
    pub const fn aircraft(&self) -> Id<AircraftType> {
        self.aircraft
    }

    /// Passenger, cargo or overflight.
    pub const fn kind(&self) -> FlightKind {
        self.kind
    }

    /// Departure airport.
    pub const fn origin(&self) -> Id<Airport> {
        self.origin
    }

    /// Arrival airport.
    pub const fn destination(&self) -> Id<Airport> {
        self.destination
    }

    /// What the aircraft is doing.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Current position.
    pub const fn position(&self) -> LatLon {
        self.position
    }

    /// True course over the ground, `[0, 360)`.
    pub const fn heading(&self) -> Degrees {
        self.heading
    }

    /// Altitude above mean sea level.
    pub const fn altitude(&self) -> Feet {
        self.altitude
    }

    /// Speed over the ground.
    pub const fn ground_speed(&self) -> Knots {
        self.ground_speed
    }

    /// Scheduled off-block time.
    pub const fn scheduled_departure(&self) -> SimTime {
        self.scheduled_departure
    }

    /// Scheduled on-block time.
    pub const fn scheduled_arrival(&self) -> SimTime {
        self.scheduled_arrival
    }

    /// Actual off-block time, or the expected one while boarding.
    pub const fn off_block(&self) -> SimTime {
        self.off_block
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_callsign_display() -> Result<()> {
        let callsign = Callsign::new("DLH".parse()?, FlightNumber::new(400));
        assert_eq!(callsign.to_string(), "DLH400");
        assert_eq!(callsign.number(), FlightNumber::new(400));
        Ok(())
    }
}
