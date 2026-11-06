//! A leg's ground parts on a mapped airport (plan M4 D71, D72): the taxi
//! route from the stand to the runway entry with its pushback, and the exit
//! a landing makes.

use crate::{
    Id, LatLon,
    geo::Polyline,
    ground::{AirportGround, EdgeKind, Exit, GroundRoute, Node, RAPID_EXIT, Stand},
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
/// Speed at which other exits are taken, m/s.
pub(crate) const EXIT_SPEED: f64 = 8.0;

/// A departure's route from its stand to the runway.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Departure {
    pub(crate) taxi: Polyline,
    /// Where the take-off roll starts, on the centreline.
    pub(crate) entry: LatLon,
    /// Length of the pushback at the start of `taxi`, m.
    pub(crate) pushback: f64,
    /// Heading of the aircraft on its stand.
    pub(crate) stand_heading: Degrees,
}

/// Route from `stand` to the entry of `runway` for an aircraft of `wingspan`;
/// `None` if there is no entry or no route.
pub(crate) fn departure(
    ground: &AirportGround,
    stand: Id<Stand>,
    runway: RunwayDesignator,
    wingspan: Meters,
) -> Option<Departure> {
    let stand = ground.stands().get(stand.as_usize())?;
    let entry = ground.departure_entry(runway)?;
    let route = ground.route(stand.node(), entry.node(), wingspan)?;
    let taxi = Polyline::new(route.points()).ok()?;
    Some(Departure {
        pushback: pushback_length(ground, &route, &taxi, stand.heading()),
        entry: ground.nodes()[entry.node().as_usize()].position(),
        taxi,
        stand_heading: stand.heading(),
    })
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
        let network = GroundNetwork::from_json(
            include_str!("../../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let id = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        Ok((world, network, id))
    }

    #[test]
    fn test_departures_push_back_from_contact_stands() -> Result<()> {
        let (_, network, id) = frankfurt()?;
        let ground = network.airport(id).context("FRA layout")?;
        let span = Meters::new(35.8)?;
        let mut pushed = 0;
        let mut planned = 0;
        for index in 0..ground.stands().len() {
            let stand = Id::from_usize(index)?;
            let Some(plan) = departure(ground, stand, "25C".parse()?, span) else {
                continue;
            };
            planned += 1;
            let start = plan.taxi.point_at(Meters::default());
            let at = ground.nodes()[ground.stands()[index].node().as_usize()].position();
            assert!(start.distance_to(at).value() < 0.5);
            let end = plan.taxi.point_at(plan.taxi.length());
            assert!(end.distance_to(plan.entry).value() < 0.5);
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
