//! One airport's validated ground network (plan M4 D67).

use std::collections::HashSet;

use anyhow::{Context, Result};

use super::{
    AirportGroundDefinition, EdgeKind, GroundName,
    definition::{EdgeDefinition, Point, StandDefinition},
};
use crate::{
    CoreError, Id, LatLon,
    units::{Degrees, Meters},
    world::{Airport, RunwayDesignator, RunwayEnd, World},
};

/// A junction or end point of the network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    position: LatLon,
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
    from: Id<Node>,
    to: Id<Node>,
    kind: EdgeKind,
    name: Option<GroundName>,
    oneway: bool,
    max_wingspan: Option<Meters>,
    /// The whole polyline, both nodes included.
    points: Vec<LatLon>,
    length: Meters,
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
    name: GroundName,
    node: Id<Node>,
    heading: Degrees,
    max_wingspan: Meters,
    area: Option<GroundName>,
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
}

/// Where departures from a runway end wait before lining up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldingPosition {
    node: Id<Node>,
    runway: RunwayDesignator,
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
    runway: RunwayDesignator,
    node: Id<Node>,
    distance: Meters,
    angle: Degrees,
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

/// An airport's taxi network with stands, holding positions and exits.
#[derive(Debug, Clone, PartialEq)]
pub struct AirportGround {
    airport: Id<Airport>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    /// `links[link_starts[n]..link_starts[n + 1]]` are the edges at node `n`.
    link_starts: Vec<u32>,
    links: Vec<Id<Edge>>,
    stands: Vec<Stand>,
    holding_positions: Vec<HoldingPosition>,
    exits: Vec<Exit>,
    aprons: Vec<Vec<LatLon>>,
}

/// Checks a definition's numbers against the airport, for error messages.
struct Checker<'a> {
    ident: &'a str,
    nodes: usize,
}

impl Checker<'_> {
    fn fail(&self, reason: &'static str) -> anyhow::Error {
        CoreError::InvalidGround {
            airport: self.ident.to_owned(),
            reason,
        }
        .into()
    }

    fn node(&self, index: u32, reason: &'static str) -> Result<Id<Node>> {
        if usize::try_from(index).is_ok_and(|i| i < self.nodes) {
            Ok(Id::new(index))
        } else {
            Err(self.fail(reason))
        }
    }

    fn point(&self, [lat, lon]: Point) -> Result<LatLon> {
        LatLon::new(lat, lon).map_err(|_| self.fail("a point is not a valid coordinate"))
    }

    fn positive(&self, value: Meters, reason: &'static str) -> Result<Meters> {
        if value.value().is_finite() && value.value() > 0.0 {
            Ok(value)
        } else {
            Err(self.fail(reason))
        }
    }
}

impl AirportGround {
    /// Validates `definition` against the world: every index resolves, every
    /// runway designator names one of the airport's runway ends.
    pub fn new(definition: AirportGroundDefinition, world: &World) -> Result<Self> {
        let ident = definition.airport.to_string();
        let airport = world.airport_by_ident(definition.airport).ok_or_else(|| {
            CoreError::UnknownAirport {
                ident: ident.clone(),
            }
        })?;
        let check = Checker {
            ident: &ident,
            nodes: definition.nodes.len(),
        };
        let nodes = definition
            .nodes
            .iter()
            .map(|&point| check.point(point).map(|position| Node { position }))
            .collect::<Result<Vec<_>>>()?;
        let edges = definition
            .edges
            .iter()
            .map(|edge| Self::build_edge(&check, &nodes, edge))
            .collect::<Result<Vec<_>>>()?;
        let (link_starts, links) = Self::links(nodes.len(), &edges)?;

        let stands = Self::build_stands(&check, definition.stands)?;
        let ends: Vec<RunwayDesignator> = world
            .runways_of(airport)
            .iter()
            .flat_map(|runway| runway.ends().iter().map(RunwayEnd::designator))
            .collect();
        let runway_end = |runway: RunwayDesignator| -> Result<RunwayDesignator> {
            if ends.contains(&runway) {
                Ok(runway)
            } else {
                Err(CoreError::UnknownRunway {
                    airport: ident.clone(),
                    runway: runway.to_string(),
                }
                .into())
            }
        };
        let holding_positions = definition
            .holding_positions
            .into_iter()
            .map(|holding| {
                Ok(HoldingPosition {
                    node: check
                        .node(holding.node, "a holding position references a missing node")?,
                    runway: runway_end(holding.runway)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let exits = definition
            .exits
            .into_iter()
            .map(|exit| {
                let angle = exit.angle.value();
                if !(0.0..=180.0).contains(&angle) || !exit.distance.value().is_finite() {
                    return Err(check.fail("an exit's angle or distance is out of range"));
                }
                Ok(Exit {
                    runway: runway_end(exit.runway)?,
                    node: check.node(exit.node, "an exit references a missing node")?,
                    distance: exit.distance,
                    angle: exit.angle,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let aprons = definition
            .aprons
            .into_iter()
            .map(|ring| {
                ring.into_iter()
                    .map(|point| check.point(point))
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            airport,
            nodes,
            edges,
            link_starts,
            links,
            stands,
            holding_positions,
            exits,
            aprons,
        })
    }

    fn build_stands(check: &Checker<'_>, definitions: Vec<StandDefinition>) -> Result<Vec<Stand>> {
        let mut names = HashSet::with_capacity(definitions.len());
        let mut stands = Vec::with_capacity(definitions.len());
        for stand in definitions {
            if !names.insert(stand.name) {
                return Err(check.fail("two stands share a name"));
            }
            if !stand.heading.value().is_finite() {
                return Err(check.fail("a stand heading is not finite"));
            }
            stands.push(Stand {
                name: stand.name,
                node: check.node(stand.node, "a stand references a missing node")?,
                heading: stand.heading.normalized(),
                max_wingspan: check.positive(
                    stand.max_wingspan,
                    "a stand's wingspan limit is not positive",
                )?,
                area: stand.area,
            });
        }
        Ok(stands)
    }

    fn build_edge(check: &Checker<'_>, nodes: &[Node], edge: &EdgeDefinition) -> Result<Edge> {
        let from = check.node(edge.from, "an edge references a missing node")?;
        let to = check.node(edge.to, "an edge references a missing node")?;
        if from == to {
            return Err(check.fail("an edge starts and ends at the same node"));
        }
        if let Some(span) = edge.max_wingspan {
            check.positive(span, "an edge's wingspan limit is not positive")?;
        }
        let mut points = Vec::with_capacity(edge.points.len() + 2);
        points.push(nodes[from.as_usize()].position);
        for &point in &edge.points {
            points.push(check.point(point)?);
        }
        points.push(nodes[to.as_usize()].position);
        let length = points
            .windows(2)
            .map(|pair| pair[0].distance_to(pair[1]).value())
            .sum();
        Ok(Edge {
            from,
            to,
            kind: edge.kind,
            name: edge.name,
            oneway: edge.oneway,
            max_wingspan: edge.max_wingspan,
            points,
            length: Meters::from_raw(length),
        })
    }

    /// Compressed adjacency: for each node, the edges touching it, in edge
    /// order.
    fn links(nodes: usize, edges: &[Edge]) -> Result<(Vec<u32>, Vec<Id<Edge>>)> {
        let mut counts = vec![0_u32; nodes + 1];
        for edge in edges {
            counts[edge.from.as_usize() + 1] += 1;
            counts[edge.to.as_usize() + 1] += 1;
        }
        for i in 1..counts.len() {
            counts[i] += counts[i - 1];
        }
        let mut fill: Vec<u32> = counts.clone();
        let mut links = vec![Id::new(0); edges.len() * 2];
        for (index, edge) in edges.iter().enumerate() {
            let id = Id::from_usize(index).context("too many ground edges")?;
            for node in [edge.from, edge.to] {
                let slot = &mut fill[node.as_usize()];
                links[*slot as usize] = id;
                *slot += 1;
            }
        }
        Ok((counts, links))
    }

    /// The airport.
    pub const fn airport(&self) -> Id<Airport> {
        self.airport
    }

    /// All nodes.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// All edges.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The edges touching `node`.
    pub fn edges_at(&self, node: Id<Node>) -> &[Id<Edge>] {
        let start = self.link_starts.get(node.as_usize()).copied().unwrap_or(0);
        let end = self
            .link_starts
            .get(node.as_usize() + 1)
            .copied()
            .unwrap_or(start);
        self.links
            .get(start as usize..end as usize)
            .unwrap_or_default()
    }

    /// The edge with id `edge`, if any.
    pub fn edge(&self, edge: Id<Edge>) -> Option<&Edge> {
        self.edges.get(edge.as_usize())
    }

    /// All stands.
    pub fn stands(&self) -> &[Stand] {
        &self.stands
    }

    /// All runway holding positions.
    pub fn holding_positions(&self) -> &[HoldingPosition] {
        &self.holding_positions
    }

    /// All runway exits.
    pub fn exits(&self) -> &[Exit] {
        &self.exits
    }

    /// Apron outlines.
    pub fn aprons(&self) -> &[Vec<LatLon>] {
        &self.aprons
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{ground::definition::tests::sample, test_data::data};

    fn world() -> Result<World> {
        Ok(data()?.world)
    }

    fn rejection(definition: AirportGroundDefinition) -> Result<CoreError> {
        let error = AirportGround::new(definition, &world()?)
            .err()
            .context("must be rejected")?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    #[test]
    fn test_airport_ground_builds_from_a_definition() -> Result<()> {
        let world = world()?;
        let ground = AirportGround::new(sample()?, &world)?;
        assert_eq!(
            Some(ground.airport()),
            world.airport_by_ident("EDDF".parse()?)
        );
        assert_eq!(ground.nodes().len(), 3);
        let taxiway = ground.edge(Id::new(1)).context("edge 1")?;
        assert_eq!(taxiway.kind(), EdgeKind::Taxiway);
        assert_eq!(taxiway.name().map(|n| n.to_string()), Some("N7".to_owned()));
        assert_eq!(taxiway.points().len(), 3);
        // Two short stretches of about 111 m each.
        assert!(
            (taxiway.length().value() - 222.4).abs() < 1.0,
            "{:?}",
            taxiway.length()
        );
        // Node 1 joins the lead-in and the taxiway.
        assert_eq!(ground.edges_at(Id::new(1)), &[Id::new(0), Id::new(1)]);
        assert_eq!(ground.edges_at(Id::new(0)), &[Id::new(0)]);
        assert_eq!(ground.edges_at(Id::new(9)), &[] as &[Id<Edge>]);
        let stand = ground.stands()[0];
        assert_eq!(stand.name().as_str(), "A14");
        assert_eq!(stand.node(), Id::new(0));
        assert_eq!(stand.area().map(|a| a.to_string()), Some("T1".to_owned()));
        assert_eq!(ground.holding_positions()[0].runway(), "25C".parse()?);
        assert_eq!(ground.exits()[0].node(), Id::new(2));
        assert_eq!(ground.aprons()[0].len(), 3);
        Ok(())
    }

    #[test]
    fn test_airport_ground_rejects_bad_references() -> Result<()> {
        let mut definition = sample()?;
        definition.edges[1].to = 3;
        assert_eq!(
            rejection(definition)?,
            CoreError::InvalidGround {
                airport: "EDDF".to_owned(),
                reason: "an edge references a missing node"
            }
        );
        let mut definition = sample()?;
        definition.edges[1].to = 1;
        assert!(matches!(
            rejection(definition)?,
            CoreError::InvalidGround { .. }
        ));
        let mut definition = sample()?;
        definition.stands[0].node = 5;
        assert!(matches!(
            rejection(definition)?,
            CoreError::InvalidGround { .. }
        ));
        let mut definition = sample()?;
        definition.stands.push(definition.stands[0].clone());
        assert_eq!(
            rejection(definition)?,
            CoreError::InvalidGround {
                airport: "EDDF".to_owned(),
                reason: "two stands share a name"
            }
        );
        let mut definition = sample()?;
        definition.stands[0].max_wingspan = Meters::from_raw(0.0);
        assert!(matches!(
            rejection(definition)?,
            CoreError::InvalidGround { .. }
        ));
        let mut definition = sample()?;
        definition.nodes[0] = [95.0, 8.0];
        assert!(matches!(
            rejection(definition)?,
            CoreError::InvalidGround { .. }
        ));
        let mut definition = sample()?;
        definition.exits[0].angle = Degrees::from_raw(200.0);
        assert!(matches!(
            rejection(definition)?,
            CoreError::InvalidGround { .. }
        ));
        Ok(())
    }

    #[test]
    fn test_airport_ground_checks_runways_and_airport() -> Result<()> {
        let mut definition = sample()?;
        definition.holding_positions[0].runway = "09".parse()?;
        assert_eq!(
            rejection(definition)?,
            CoreError::UnknownRunway {
                airport: "EDDF".to_owned(),
                runway: "09".to_owned()
            }
        );
        let mut definition = sample()?;
        definition.exits[0].runway = "04".parse()?;
        assert!(matches!(
            rejection(definition)?,
            CoreError::UnknownRunway { .. }
        ));
        let mut definition = sample()?;
        definition.airport = "EDXX".parse()?;
        assert!(matches!(
            rejection(definition)?,
            CoreError::UnknownAirport { .. }
        ));
        Ok(())
    }
}
