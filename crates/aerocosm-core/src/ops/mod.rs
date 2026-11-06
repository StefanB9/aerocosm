//! Flight operations: tails flying their weekly rotations, phase by phase
//! (plan M2, PR 3).

mod control;
mod fleet;
// Temporary until operations hold aircraft (removed in that commit).
#[cfg_attr(not(test), allow(dead_code))]
mod hold;
mod motion;
mod path;
mod procedure;
mod runway;
mod tail;
mod timeline;
mod view;

pub(crate) use fleet::Operations;
pub use fleet::board::{BoardEntry, BoardKind, FlightStatus};
pub use view::{Callsign, FlightView};

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

impl Phase {
    /// Climb, cruise, descent or approach: off the ground.
    pub const fn is_airborne(self) -> bool {
        matches!(
            self,
            Self::Climb | Self::Cruise | Self::Descent | Self::Approach
        )
    }
}

/// How a run starts (D44).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartMode {
    /// Every tail at its nominal position at the start instant.
    #[default]
    Warm,
    /// Only legs departing after the start fly; tails wait at their next
    /// origin.
    Cold,
}
