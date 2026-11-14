//! The parts of a ground network: nodes, edges, stands, holding positions
//! and exits.

use super::{EdgeKind, GroundName};
use crate::{
    Id, LatLon,
    units::{Degrees, Meters},
    world::RunwayDesignator,
};

/// A junction or end point of the network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub(super) position: LatLon,
    /// East and north of the airport's first node, m: for route estimates.
    pub(super) xy: [f64; 2],
}

impl Node {
    /// Where it is.
    pub const fn position(&self) -> LatLon {
        self.position
    }
}

/// A stretch of pavement between two nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub(super) from: Id<Node>,
    pub(super) to: Id<Node>,
    pub(super) kind: EdgeKind,
    pub(super) name: Option<GroundName>,
    pub(super) oneway: bool,
    pub(super) max_wingspan: Option<Meters>,
    /// The whole polyline, both nodes included.
    pub(super) points: Vec<LatLon>,
    pub(super) length: Meters,
}

impl Edge {
    /// Start node.
    pub const fn from(&self) -> Id<Node> {
        self.from
    }

    /// End node.
    pub const fn to(&self) -> Id<Node> {
        self.to
    }

    /// Pavement kind.
    pub const fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// Taxiway or runway name, if mapped.
    pub const fn name(&self) -> Option<GroundName> {
        self.name
    }

    /// Only to be taxied from [`Self::from`] to [`Self::to`].
    pub const fn oneway(&self) -> bool {
        self.oneway
    }

    /// Largest wingspan allowed, where published.
    pub const fn max_wingspan(&self) -> Option<Meters> {
        self.max_wingspan
    }

    /// The polyline from [`Self::from`] to [`Self::to`].
    pub fn points(&self) -> &[LatLon] {
        &self.points
    }

    /// Length along the polyline.
    pub const fn length(&self) -> Meters {
        self.length
    }
}

/// A parking stand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stand {
    pub(super) name: GroundName,
    pub(super) node: Id<Node>,
    pub(super) heading: Degrees,
    pub(super) max_wingspan: Meters,
    pub(super) area: Option<GroundName>,
    pub(super) cargo: bool,
}

impl Stand {
    /// Designation, e.g. "V155".
    pub const fn name(&self) -> GroundName {
        self.name
    }

    /// The node a parked aircraft stands on.
    pub const fn node(&self) -> Id<Node> {
        self.node
    }

    /// True heading of a parked aircraft.
    pub const fn heading(&self) -> Degrees {
        self.heading
    }

    /// Largest wingspan that fits.
    pub const fn max_wingspan(&self) -> Meters {
        self.max_wingspan
    }

    /// Apron or terminal, if known.
    pub const fn area(&self) -> Option<GroundName> {
        self.area
    }

    /// On an apron mapped for cargo.
    pub const fn cargo(&self) -> bool {
        self.cargo
    }
}

/// Where departures from a runway end wait before lining up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldingPosition {
    pub(super) node: Id<Node>,
    pub(super) runway: RunwayDesignator,
}

impl HoldingPosition {
    /// The node on the taxiway.
    pub const fn node(&self) -> Id<Node> {
        self.node
    }

    /// The runway end departures line up at.
    pub const fn runway(&self) -> RunwayDesignator {
        self.runway
    }
}

/// Where landings on a runway end can turn off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exit {
    pub(super) runway: RunwayDesignator,
    pub(super) node: Id<Node>,
    pub(super) distance: Meters,
    pub(super) angle: Degrees,
}

impl Exit {
    /// The landing runway end.
    pub const fn runway(&self) -> RunwayDesignator {
        self.runway
    }

    /// The node on the runway centreline.
    pub const fn node(&self) -> Id<Node> {
        self.node
    }

    /// Distance from the landing threshold.
    pub const fn distance(&self) -> Meters {
        self.distance
    }

    /// Turn off the landing direction.
    pub const fn angle(&self) -> Degrees {
        self.angle
    }
}

/// The stand a variant belongs to: `05A` → `05`, `164Y` → `164`, and the
/// import's own `N4-2` → `N4`. Variants overlap and share their stand's
/// spacing.
pub fn stand_base(name: &str) -> &str {
    let name = match name.rsplit_once('-') {
        Some((base, suffix)) if !base.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) => {
            base
        }
        _ => name,
    };
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if trimmed.ends_with(|c: char| c.is_ascii_digit()) {
        trimmed
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stand_base_strips_variant_letters_and_suffixes() {
        assert_eq!(stand_base("05A"), "05");
        assert_eq!(stand_base("164Y"), "164");
        assert_eq!(stand_base("V 117"), "V 117");
        assert_eq!(stand_base("N4-2"), "N4");
        assert_eq!(stand_base("A"), "A");
        assert_eq!(stand_base("A14L"), "A14");
    }
}
