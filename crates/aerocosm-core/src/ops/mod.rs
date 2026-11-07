//! Flight operations: tails flying their weekly rotations, phase by phase
//! (plan M2, PR 3).

mod control;
mod fleet;
mod hold;
mod motion;
mod path;
mod procedure;
mod runway;
mod stats;
mod tail;
mod timeline;
mod view;

pub(crate) use fleet::Operations;
pub use fleet::board::{BoardEntry, BoardKind, FlightStatus};
pub use hold::{PATTERN_POINTS, holding_pattern};
pub use runway::ActiveRunway;

/// The runway ends in use with the prevailing wind (fixed until weather
/// arrives): those of the airport's operating mode for it, else the
/// best-aligned end in mixed mode.
pub fn active_runways_now(
    runways: &[crate::world::Runway],
    modes: &[crate::traffic::RunwayMode],
) -> Vec<ActiveRunway> {
    runway::active_runways(runways, modes, runway::PREVAILING_WIND)
}
pub use stats::{AirportStats, Statistics};
pub use view::{Callsign, FlightView, Holding};

/// What an aircraft is doing on a leg, in the order it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// At the gate before departure; boarding for the next leg.
    Boarding,
    /// Pushed back from the stand and starting engines (plan M4 D72).
    Pushback,
    /// Taxiing from the gate to the departure runway.
    TaxiOut,
    /// At the runway holding point, waiting for take-off clearance.
    Queue,
    /// Cleared for take-off, taxiing onto the runway (plan M4 D78).
    LineUp,
    /// Accelerating along the runway.
    TakeoffRoll,
    /// Climbing towards the leg's cruise altitude.
    Climb,
    /// Level at cruise altitude.
    Cruise,
    /// Flying a holding pattern before the approach.
    Holding,
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
            Self::Climb | Self::Cruise | Self::Holding | Self::Descent | Self::Approach
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

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::test_data::data;

    #[test]
    fn test_active_runways_now_uses_the_west_flow() -> Result<()> {
        let data = data()?;
        let frankfurt = data
            .world
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let runways = data.world.runways_of(frankfurt);
        let active = active_runways_now(runways, data.traffic.runway_modes("EDDF".parse()?));
        let names: Vec<String> = active
            .iter()
            .map(|a| runways[a.runway].ends()[a.end].designator().to_string())
            .collect();
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(names.contains(&"25R".to_owned()) && names.contains(&"18".to_owned()));
        let single = active_runways_now(runways, &[]);
        assert_eq!(single.len(), 1);
        assert!(single[0].arrivals && single[0].departures);
        Ok(())
    }
}
