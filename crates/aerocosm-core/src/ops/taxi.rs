//! What ground control is doing with an aircraft on a mapped airport's
//! ground (plan M4 D85), as the flight panel tells it.

use super::Callsign;
use crate::{LatLon, geo::Polyline, ground::GroundName, units::Meters, world::RunwayDesignator};

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
    /// Cleared to follow another aircraft going the same way (D89).
    Following {
        /// The flight it follows.
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

/// A taxiing aircraft's route (plan M4 D86): the polyline it taxies, how far
/// along it is, how far it is cleared, and where the route ends, all in
/// metres from the polyline's start.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxiRoute {
    points: Vec<LatLon>,
    line: Polyline,
    along: Meters,
    cleared: Meters,
    end: Meters,
}

impl TaxiRoute {
    /// The route through `points`, `along` metres taxied and cleared to
    /// `cleared` of `end`; the clearance kept between the two.
    pub(crate) fn new(points: Vec<LatLon>, along: f64, cleared: f64, end: f64) -> Option<Self> {
        let line = Polyline::new(&points).ok()?;
        let end = end.max(along);
        Some(Self {
            points,
            line,
            along: Meters::from_raw(along),
            cleared: Meters::from_raw(cleared.clamp(along, end)),
            end: Meters::from_raw(end),
        })
    }

    /// The route's points, from where the taxi started.
    pub fn points(&self) -> &[LatLon] {
        &self.points
    }

    /// Where the route is `distance` from its start.
    pub fn point_at(&self, distance: Meters) -> LatLon {
        self.line.point_at(distance)
    }

    /// How far the aircraft has taxied.
    pub const fn along(&self) -> Meters {
        self.along
    }

    /// How far it is cleared to taxi.
    pub const fn cleared(&self) -> Meters {
        self.cleared
    }

    /// Where the route ends.
    pub const fn end(&self) -> Meters {
        self.end
    }
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
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_taxi_route_keeps_the_clearance_between_here_and_the_end() -> Result<()> {
        let points = vec![LatLon::new(50.03, 8.56)?, LatLon::new(50.04, 8.56)?];
        let route = TaxiRoute::new(points.clone(), 100.0, 500.0, 1_000.0).context("a route")?;
        assert_eq!(route.points(), points.as_slice());
        assert!((route.cleared().value() - 500.0).abs() < 1e-9);
        assert!((route.end().value() - 1_000.0).abs() < 1e-9);
        let start = route.point_at(Meters::default());
        assert!(start.distance_to(points[0]).value() < 0.01);
        // Nothing cleared yet reads as cleared to here; beyond the end, to it.
        let held =
            TaxiRoute::new(points.clone(), 100.0, f64::NEG_INFINITY, 1_000.0).context("a route")?;
        assert!((held.cleared().value() - 100.0).abs() < 1e-9);
        let done = TaxiRoute::new(points, 100.0, 2_000.0, 1_000.0).context("a route")?;
        assert!((done.cleared().value() - 1_000.0).abs() < 1e-9);
        assert!(TaxiRoute::new(Vec::new(), 0.0, 0.0, 0.0).is_none());
        Ok(())
    }

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
