//! What ground control is doing with an aircraft on a mapped airport's
//! ground (plan M4 D85), as the flight panel tells it.

use super::Callsign;
use crate::{ground::GroundName, world::RunwayDesignator};

/// How many taxiway names ahead a taxi state carries.
pub const VIA_NAMES: usize = 3;

/// Where a taxi ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxiGoal {
    /// A runway holding point, for take-off from this runway end where the
    /// layout says which.
    HoldingPoint(Option<RunwayDesignator>),
    /// A parking stand.
    Stand(GroundName),
    /// A waiting spot, until a stand frees.
    Spot,
}

/// What an aircraft on the ground of a mapped airport is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxiState {
    /// Pushing back from a stand.
    Pushback {
        /// The stand, where known.
        stand: Option<GroundName>,
    },
    /// Taxiing along the named taxiways ahead, in order, towards `to`.
    Taxiing {
        /// The next taxiway names on the route, first to come first.
        via: [Option<GroundName>; VIA_NAMES],
        /// Where the taxi ends.
        to: TaxiGoal,
    },
    /// Holding short of a runway until it may cross.
    HoldingShort {
        /// The runway, where named.
        runway: Option<GroundName>,
    },
    /// Holding for another aircraft in the way.
    WaitingFor {
        /// The flight it waits for.
        flight: Callsign,
        /// Where the taxi ends.
        to: TaxiGoal,
    },
    /// At the holding point, waiting to line up.
    AtHoldingPoint {
        /// The runway end it takes off from, where the layout says which.
        runway: Option<RunwayDesignator>,
    },
    /// At a waiting spot until a stand frees.
    WaitingForStand,
    /// Taxiing ignoring others after a deadlock or a long wait (D75).
    ClearedRegardless {
        /// Where the taxi ends.
        to: TaxiGoal,
    },
}

/// The first `VIA_NAMES` distinct names in `names`, skipping unnamed
/// stretches and repeats of the name just taken.
pub(crate) fn via_names(
    names: impl IntoIterator<Item = Option<GroundName>>,
) -> [Option<GroundName>; VIA_NAMES] {
    let mut via = [None; VIA_NAMES];
    let mut count = 0;
    for name in names.into_iter().flatten() {
        if count > 0 && via[count - 1] == Some(name) {
            continue;
        }
        if count == VIA_NAMES {
            break;
        }
        via[count] = Some(name);
        count += 1;
    }
    via
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_via_names_takes_the_first_distinct_names_in_order() -> Result<()> {
        let (n, n7, l, b) = ("N".parse()?, "N7".parse()?, "L".parse()?, "B".parse()?);
        let names = [
            None,
            Some(n),
            Some(n),
            None,
            Some(n7),
            Some(n7),
            Some(l),
            Some(b),
        ];
        assert_eq!(via_names(names), [Some(n), Some(n7), Some(l)]);
        assert_eq!(via_names([None, Some(n)]), [Some(n), None, None]);
        assert_eq!(via_names([]), [None; VIA_NAMES]);
        // A name met again later counts again: the route comes back to it.
        assert_eq!(
            via_names([Some(n), Some(l), Some(n)]),
            [Some(n), Some(l), Some(n)]
        );
        Ok(())
    }
}
