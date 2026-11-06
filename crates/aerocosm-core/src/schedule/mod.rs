//! Schedule generation: a deterministic weekly template of flights grouped into
//! tails (plan M2, PR 2).

// Temporary while the stages are added one commit at a time: they are crate-internal and only
// become reachable once `generate` composes them (removed in that commit).
#![cfg_attr(not(test), allow(dead_code))]

mod demand;
mod network;
mod overflight;
mod rotation;
mod timetable;
mod trips;

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

impl std::fmt::Display for FlightNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
