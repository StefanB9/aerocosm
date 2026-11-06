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
