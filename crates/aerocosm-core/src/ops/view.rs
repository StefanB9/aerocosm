//! Read-only views of active flights.

use std::fmt;

use super::{Phase, TaxiState};
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

/// A holding pattern being flown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Holding {
    pub(super) fix: LatLon,
    pub(super) inbound: Degrees,
    pub(super) until: Option<SimTime>,
}

impl Holding {
    /// The holding fix.
    pub const fn fix(&self) -> LatLon {
        self.fix
    }

    /// Inbound course to the fix.
    pub const fn inbound(&self) -> Degrees {
        self.inbound
    }

    /// Expected time out of the hold; `None` while waiting at the final fix
    /// for the approach.
    pub const fn until(&self) -> Option<SimTime> {
        self.until
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
    pub(super) holding: Option<Holding>,
    pub(super) slot_time: Option<SimTime>,
    pub(super) expected_landing: Option<SimTime>,
    pub(super) speed_factor: f64,
    pub(super) ground_conflict: bool,
    pub(super) taxi: Option<TaxiState>,
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

    /// Passenger or cargo.
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

    /// The holding pattern, while holding.
    pub const fn holding(&self) -> Option<Holding> {
        self.holding
    }

    /// Calculated take-off time while held at the gate for a congested
    /// destination (D58).
    pub const fn slot_time(&self) -> Option<SimTime> {
        self.slot_time
    }

    /// Landing time booked by the arrival manager, once sequenced.
    pub const fn expected_landing(&self) -> Option<SimTime> {
        self.expected_landing
    }

    /// Fraction of normal speed flown under speed control (1 without).
    pub const fn speed_factor(&self) -> f64 {
        self.speed_factor
    }

    /// Whether it taxies on a route planned around nobody, after ground
    /// control found no conflict-free one (plan M4 D75).
    pub const fn ground_conflict(&self) -> bool {
        self.ground_conflict
    }

    /// What ground control is doing with it, while it pushes back, taxies
    /// or waits at the holding point of a mapped airport (plan M4 D85).
    pub const fn taxi(&self) -> Option<TaxiState> {
        self.taxi
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
