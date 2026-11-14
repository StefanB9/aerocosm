//! A leg's ground parts on a mapped airport (plan M4 D71, D72): the taxi
//! route from the stand to the runway entry with its pushback, and the exit
//! a landing makes.

use crate::{
    Id, LatLon,
    geo::Polyline,
    ground::{
        AirportGround, Edge, EdgeKind, Exit, GroundRoute, HoldingPosition, Node, RAPID_EXIT, Stand,
    },
    traffic::angle_between,
    units::{Degrees, Meters},
    world::RunwayDesignator,
};

/// Touchdown this far past the threshold, m, when choosing an exit.
const TOUCHDOWN: f64 = 300.0;
/// Braking deceleration after touchdown, m/s² (as in motion).
const BRAKING: f64 = 1.8;
/// Speed at which a rapid exit is taken, m/s (about 30 kt).
pub(crate) const RAPID_EXIT_SPEED: f64 = 15.0;
/// Departures queue at most this far from the runway entry, m: lining up
/// occupies the runway (plan M4 D78).
const MAX_LINEUP: f64 = 150.0;
/// and at least this far, to stay clear of it.
const MIN_LINEUP: f64 = 60.0;
/// Speed at which other exits are taken, m/s.
pub(crate) const EXIT_SPEED: f64 = 8.0;

/// A departure's route from its stand to the runway.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Departure {
    /// From the stand to the runway holding position.
    pub(crate) taxi: Polyline,
    /// From the holding position onto the runway (plan M4 D78).
    pub(crate) lineup: Polyline,
    /// The holding position node, where departures queue.
    pub(crate) hold: Id<Node>,
    /// From the holding position to where the line-up starts, when the
    /// holding position is far from the runway (taxied, not reserved).
    pub(crate) queue_points: Vec<LatLon>,
    /// Where the pushback ends and taxiing starts (the stand without one).
    pub(crate) joint: Id<Node>,
    /// Where the take-off roll starts, on the centreline.
    pub(crate) entry: LatLon,
    /// Length of the pushback at the start of `taxi`, m.
    pub(crate) pushback: f64,
    /// Heading of the aircraft on its stand.
    pub(crate) stand_heading: Degrees,
}

/// Where a departure starts on the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Its stand, pushing back where the stand faces the apron.
    Stand(Id<Stand>),
    /// A node on the taxiways, after boarding in remote parking (plan M5
    /// D127): no pushback.
    Node(Id<Node>),
}

/// Route from `origin` to the entry of `runway` for an aircraft of
/// `wingspan`; `None` if there is no entry or no route.
pub(crate) fn departure(
    ground: &AirportGround,
    origin: Origin,
    runway: (RunwayDesignator, LatLon),
    wingspan: Meters,
) -> Option<Departure> {
    let (runway, threshold) = runway;
    let (from, parked) = match origin {
        Origin::Stand(stand) => {
            let stand = ground.stands().get(stand.as_usize())?;
            (stand.node(), Some(stand.heading()))
        }
        Origin::Node(node) => (node, None),
    };
    let entry = ground.departure_entry(runway, threshold)?;
    let route = ground.route(from, entry.node(), wingspan)?;
    let whole = Polyline::new(route.points()).ok()?;
    let pushback = parked.map_or(0.0, |heading| {
        pushback_length(ground, &route, &whole, heading)
    });
    // From a node it faces along the route.
    let heading = parked.or_else(|| {
        let points = route.points();
        Degrees::new(points.first()?.initial_bearing_to(*points.get(1)?).value()).ok()
    })?;
    // Queue at the last holding position of this runway on the way, else
    // one node before the entry.
    let nodes = route_nodes(ground, from, route.edges());
    let holds: Vec<Id<Node>> = ground
        .holding_positions()
        .iter()
        .filter(|hold| hold.runway() == runway)
        .map(HoldingPosition::node)
        .collect();
    // Distance left to the entry from each node of the route.
    let mut left = vec![0.0; nodes.len()];
    for (i, &(edge, _)) in route.edges().iter().enumerate().rev() {
        left[i] = left[i + 1] + ground.edge(edge).map_or(0.0, |e| e.length().value());
    }
    let near = |i: usize| left[i] <= MAX_LINEUP;
    let split = nodes
        .iter()
        .enumerate()
        .rposition(|(i, node)| holds.contains(node) && near(i))
        .or_else(|| (0..nodes.len()).rev().find(|&i| left[i] >= MIN_LINEUP))
        .unwrap_or_else(|| nodes.len().saturating_sub(2));
    let (to_hold, onto) = route.edges().split_at(split.min(route.edges().len()));
    let start = ground.nodes()[from.as_usize()].position();
    let hold = nodes.get(split).copied().unwrap_or(from);
    let hold_point = ground.nodes()[hold.as_usize()].position();
    let pushed = if pushback > 0.0 {
        route
            .edges()
            .iter()
            .take_while(|(edge, _)| {
                ground
                    .edge(*edge)
                    .is_some_and(|edge| edge.kind() == EdgeKind::LeadIn)
            })
            .count()
            .min(split)
    } else {
        0
    };
    // A long way from the holding position: taxi on, line up only the
    // last stretch.
    let onto_points = edge_points(ground, hold_point, onto);
    let total: f64 = onto_points
        .windows(2)
        .map(|w| w[0].distance_to(w[1]).value())
        .sum();
    let (queue_points, lineup_points) = if total > MAX_LINEUP {
        split_points(&onto_points, total - MAX_LINEUP)
    } else {
        (vec![hold_point], onto_points)
    };
    let mut taxi_points = edge_points(ground, start, to_hold);
    taxi_points.extend(queue_points.iter().skip(1));
    Some(Departure {
        joint: nodes.get(pushed).copied().unwrap_or(from),
        taxi: Polyline::new(&taxi_points).ok()?,
        lineup: Polyline::new(&lineup_points).ok()?,
        queue_points,
        hold,
        pushback,
        entry: ground.nodes()[entry.node().as_usize()].position(),
        stand_heading: heading,
    })
}

/// `points` cut at `distance` along: the part before (ending at the cut)
/// and the part after (starting at it).
pub(crate) fn split_points(points: &[LatLon], distance: f64) -> (Vec<LatLon>, Vec<LatLon>) {
    let mut walked = 0.0;
    for (i, pair) in points.windows(2).enumerate() {
        let step = pair[0].distance_to(pair[1]).value();
        if walked + step >= distance && step > 0.0 {
            let cut = pair[0]
                .interpolate(pair[1], ((distance - walked) / step).clamp(0.0, 1.0))
                .unwrap_or(pair[0]);
            let mut before = points[..=i].to_vec();
            before.push(cut);
            let mut after = vec![cut];
            after.extend_from_slice(&points[i + 1..]);
            return (before, after);
        }
        walked += step;
    }
    let last = points.last().copied();
    (points.to_vec(), last.into_iter().collect())
}

/// The nodes a route passes, from `start`.
pub(crate) fn route_nodes(
    ground: &AirportGround,
    start: Id<Node>,
    edges: &[(Id<Edge>, bool)],
) -> Vec<Id<Node>> {
    let mut nodes = Vec::with_capacity(edges.len() + 1);
    nodes.push(start);
    for &(edge, forward) in edges {
        if let Some(edge) = ground.edge(edge) {
            nodes.push(if forward { edge.to() } else { edge.from() });
        }
    }
    nodes
}

/// The polyline along `edges` from `start`.
fn edge_points(ground: &AirportGround, start: LatLon, edges: &[(Id<Edge>, bool)]) -> Vec<LatLon> {
    let mut points = Vec::with_capacity(edges.len() * 4 + 1);
    points.push(start);
    for &(edge, forward) in edges {
        let Some(edge) = ground.edge(edge) else {
            continue;
        };
        if forward {
            points.extend_from_slice(&edge.points()[1..]);
        } else {
            points.extend(edge.points().iter().rev().skip(1));
        }
    }
    points
}

/// The lead-in at the start of `route`, if the aircraft leaves backwards:
/// the route starts against the stand heading.
fn pushback_length(
    ground: &AirportGround,
    route: &GroundRoute,
    taxi: &Polyline,
    heading: Degrees,
) -> f64 {
    if taxi.length().value() < 1.0 {
        return 0.0;
    }
    let away = taxi.bearing_at(Meters::from_raw(0.5));
    if angle_between(away, heading) <= 90.0 {
        // Drive-through: it leaves forwards.
        return 0.0;
    }
    route
        .edges()
        .iter()
        .map_while(|&(edge, _)| ground.edge(edge))
        .take_while(|edge| edge.kind() == EdgeKind::LeadIn)
        .map(|edge| edge.length().value())
        .sum::<f64>()
        .min(taxi.length().value())
}

/// The exit a landing at `approach_speed` m/s makes from `runway`, with
/// the speed it is taken at.
pub(crate) fn landing_exit(
    ground: &AirportGround,
    runway: RunwayDesignator,
    approach_speed: f64,
) -> Option<(&Exit, f64)> {
    let roll = |exit_speed: f64| {
        Meters::from_raw(
            TOUCHDOWN
                + (approach_speed * approach_speed - exit_speed * exit_speed).max(0.0)
                    / (2.0 * BRAKING),
        )
    };
    let exit = ground.landing_exit(runway, roll(RAPID_EXIT_SPEED), roll(EXIT_SPEED))?;
    let speed = if exit.angle().value() <= RAPID_EXIT {
        RAPID_EXIT_SPEED
    } else {
        EXIT_SPEED
    };
    Some((exit, speed))
}

/// Route from the exit node `from` to `stand`.
pub(crate) fn arrival(
    ground: &AirportGround,
    from: Id<Node>,
    stand: Id<Stand>,
    wingspan: Meters,
) -> Option<(Polyline, Degrees)> {
    let stand = ground.stands().get(stand.as_usize())?;
    let route = ground.route(from, stand.node(), wingspan)?;
    Some((Polyline::new(route.points()).ok()?, stand.heading()))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{ground::GroundNetwork, test_data::data, world::World};

    fn frankfurt() -> Result<(World, GroundNetwork, Id<crate::world::Airport>)> {
        let world = data()?.world;
        let network = GroundNetwork::from_definition(crate::test_data::ground()?, &world)?;
        let id = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        Ok((world, network, id))
    }

    #[test]
    fn test_departures_hold_short_off_the_runway_at_munich() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_definition(crate::test_data::ground()?, &world)?;
        let id = world.airport_by_ident("EDDM".parse()?).context("EDDM")?;
        let ground = network.airport(id).context("MUC layout")?;
        let (runway, end) = world
            .runways_of(id)
            .iter()
            .find_map(|r| {
                r.ends()
                    .iter()
                    .find(|end| end.designator().to_string() == "26L")
                    .map(|end| (r, end))
            })
            .context("26L")?;
        let on_runway = ground.nodes_on(runway);
        let span = Meters::new(35.8)?;
        let mut planned = 0;
        for index in 0..ground.stands().len() {
            let stand = Id::from_usize(index)?;
            let Some(plan) = departure(
                ground,
                Origin::Stand(stand),
                ("26L".parse()?, end.threshold()),
                span,
            ) else {
                continue;
            };
            planned += 1;
            assert!(
                !on_runway.contains(&plan.hold),
                "stand {}: holds short on the runway at node {}",
                ground.stands()[index].name(),
                plan.hold.as_usize()
            );
        }
        assert_ne!(planned, 0);
        Ok(())
    }

    #[test]
    fn test_departures_push_back_from_contact_stands() -> Result<()> {
        let (world, network, id) = frankfurt()?;
        let ground = network.airport(id).context("FRA layout")?;
        let span = Meters::new(35.8)?;
        let threshold = world
            .runways_of(id)
            .iter()
            .flat_map(|r| r.ends().iter())
            .find(|end| end.designator().to_string() == "25C")
            .context("25C")?
            .threshold();
        let mut pushed = 0;
        let mut planned = 0;
        for index in 0..ground.stands().len() {
            let stand = Id::from_usize(index)?;
            let Some(plan) = departure(
                ground,
                Origin::Stand(stand),
                ("25C".parse()?, threshold),
                span,
            ) else {
                continue;
            };
            planned += 1;
            let start = plan.taxi.point_at(Meters::default());
            let at = ground.nodes()[ground.stands()[index].node().as_usize()].position();
            assert!(start.distance_to(at).value() < 0.5);
            // Taxied to the queue point, then lined up onto the entry, at
            // most 150 m.
            let end = plan.taxi.point_at(plan.taxi.length());
            let lined = plan.lineup.point_at(Meters::default());
            assert!(lined.distance_to(end).value() < 0.5);
            assert!(plan.lineup.length().value() <= MAX_LINEUP + 0.5);
            let hold = ground.nodes()[plan.hold.as_usize()].position();
            let queue_start = plan.queue_points.first().copied().context("queue")?;
            assert!(queue_start.distance_to(hold).value() < 0.5);
            let onto = plan.lineup.point_at(plan.lineup.length());
            assert!(onto.distance_to(plan.entry).value() < 0.5);
            assert!(plan.pushback >= 0.0 && plan.pushback <= plan.taxi.length().value());
            if plan.pushback > 0.0 {
                pushed += 1;
            }
        }
        assert!(planned * 10 >= ground.stands().len() * 9, "{planned}");
        assert!(
            pushed * 2 > planned,
            "most stands are nose-in: {pushed} of {planned}"
        );
        Ok(())
    }

    #[test]
    fn test_split_points_cuts_at_a_distance() -> Result<()> {
        let a = LatLon::new(50.0, 8.0)?;
        let b = LatLon::new(50.0, 8.01)?;
        let c = LatLon::new(50.01, 8.01)?;
        let ab = a.distance_to(b).value();
        let (before, after) = split_points(&[a, b, c], ab + 100.0);
        assert_eq!(before.len(), 3);
        assert_eq!(after.len(), 2);
        let cut = before[2];
        assert!((b.distance_to(cut).value() - 100.0).abs() < 0.5);
        assert!(after[0].distance_to(cut).value() < 1e-6);
        let (all, rest) = split_points(&[a, b], 1e9);
        assert_eq!((all.len(), rest.len()), (2, 1));
        Ok(())
    }

    #[test]
    fn test_landing_exits_depend_on_the_approach_speed() -> Result<()> {
        let (_, network, id) = frankfurt()?;
        let ground = network.airport(id).context("FRA layout")?;
        let runway: RunwayDesignator = "25L".parse()?;
        let (slow, _) = landing_exit(ground, runway, 60.0).context("an exit")?;
        let (fast, _) = landing_exit(ground, runway, 80.0).context("an exit")?;
        assert!(slow.distance() <= fast.distance());
        assert!(slow.distance().value() > 900.0, "{:?}", slow.distance());
        let (polyline, _) = arrival(ground, slow.node(), Id::new(0), Meters::new(35.8)?)
            .context("a route to a stand")?;
        assert!(polyline.length().value() > 100.0);
        assert!(landing_exit(ground, "09".parse()?, 70.0).is_none());
        Ok(())
    }
}
