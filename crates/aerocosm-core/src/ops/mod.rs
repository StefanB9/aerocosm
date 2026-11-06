//! Flight operations: tails flying their weekly rotations, phase by phase
//! (plan M2, PR 3).

// Temporary while the parts are added one commit at a time: they become reachable once the
// simulation runs operations (removed in that commit).
#![cfg_attr(not(test), allow(dead_code))]

mod motion;
mod path;
mod runway;
mod timeline;

/// What an aircraft is doing on a leg, in the order it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// At the gate before departure; boarding for the next leg.
    Boarding,
    /// Taxiing from the gate to the departure runway.
    TaxiOut,
    /// Accelerating along the runway.
    TakeoffRoll,
    /// Climbing towards the leg's cruise altitude.
    Climb,
    /// Level at cruise altitude.
    Cruise,
    /// Descending towards the final approach fix.
    Descent,
    /// On the final approach path to the arrival runway.
    Approach,
    /// Braking along the arrival runway.
    Landing,
    /// Taxiing from the runway to the gate.
    TaxiIn,
    /// At the arrival gate; the leg is over.
    Arrived,
}
