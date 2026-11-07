//! Planning a leg: runway ends from the operating modes, ground routes at
//! mapped airports (plan M4), procedures between them.

use anyhow::{Context, Result};

use super::{
    Leg, Path,
    ground::{departure, landing_exit},
    reverse,
};
use crate::{
    AircraftType, Id, LatLon,
    geo::{Polyline, Route, Segment},
    ground::{GroundNetwork, Node, Stand},
    ops::{
        motion::{FINAL_LENGTH, Profile, ProfileInputs},
        procedure::{arrival_points, climb_out_distance, departure_turn_point},
        runway::{PREVAILING_WIND, RunwayChoice, mode_runway, runway_in_use},
    },
    traffic::{TrafficTargets, angle_between, mode_for_wind},
    units::{Degrees, Meters},
    world::{Airport, RunwayDesignator, RunwayEnd, World},
};

/// The ground layouts a leg is planned on, and the stand it leaves from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LegGround<'a> {
    pub(crate) network: &'a GroundNetwork,
    /// The stand at a mapped origin.
    pub(crate) origin_stand: Option<Id<Stand>>,
}

/// A runway end in use: threshold, heading, runway index and designator.
type End = (LatLon, Degrees, Option<(usize, RunwayDesignator)>);

/// Runways down to this fraction of the type's take-off field length are
/// used, as in schedule generation.
const RUNWAY_MARGIN: f64 = 0.75;
/// Departures prefer the parallel runway nearest a point this far along
/// the course, m.
const SIDE_PROBE: f64 = 5_000.0;
/// Turn radius of procedures: 210 kt at 25° of bank.
const TURN_RADIUS: f64 = 2_550.0;

/// Take-off or landing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    Departure,
    Arrival,
}

/// The runway end used at `airport`: from its active operating mode, scored
/// by `score`, or else the best-aligned long-enough runway; the airport
/// point and `fallback` course without runways.
fn runway_end(
    world: &World,
    traffic: &TrafficTargets,
    airport: Id<Airport>,
    aircraft: &AircraftType,
    use_: Use,
    score: impl Fn(&RunwayEnd) -> f64,
    fallback: Degrees,
) -> Result<End> {
    let place = world.airport(airport).context("airport id out of range")?;
    let runways = world.runways_of(airport);
    let min_length = aircraft.min_runway_length() * RUNWAY_MARGIN;
    let from_mode =
        mode_for_wind(traffic.runway_modes(place.ident()), PREVAILING_WIND).and_then(|mode| {
            let allowed = match use_ {
                Use::Departure => &mode.departures,
                Use::Arrival => &mode.arrivals,
            };
            mode_runway(runways, allowed, min_length, score)
        });
    let choice: Option<RunwayChoice> =
        from_mode.or_else(|| runway_in_use(runways, PREVAILING_WIND, min_length));
    Ok(match choice {
        Some(choice) => {
            let end = &runways[choice.runway].ends()[choice.end];
            (
                end.threshold(),
                end.true_heading(),
                Some((choice.runway, end.designator())),
            )
        }
        None => (place.location(), fallback, None),
    })
}

/// The lengths of `path`'s parts into `inputs`.
fn measure(path: &Path, inputs: &mut ProfileInputs) {
    inputs.enroute_length = path.enroute.length().value();
    inputs.taxi_out_length = path.taxi_out.length().value();
    inputs.taxi_in_length = path.taxi_in.length().value();
    if path.exit.is_some() {
        // The roll ends at the planned exit.
        inputs.landing_length = path.landing.length().value().max(1.0);
    }
}

/// Where a landing on the threshold `arrival` heading `course` stops
/// braking, without a mapped exit.
fn roll_out(arrival: LatLon, course: Degrees, profile: &Profile) -> LatLon {
    arrival.destination(course, Meters::from_raw(profile.landing_length()))
}

/// The aircraft's speeds and rates and the field elevations, in SI units.
fn profile_inputs(aircraft: &AircraftType, from: &Airport, to: &Airport) -> ProfileInputs {
    let speed = |knots: crate::units::Knots| knots.to_meters_per_second().value();
    let rate = |rate: crate::units::FeetPerMinute| rate.to_meters_per_second().value();
    ProfileInputs {
        cruise_speed: speed(aircraft.cruise_speed()),
        cruise_altitude: aircraft.cruise_altitude().to_meters().value(),
        climb_rate: rate(aircraft.climb_rate()),
        descent_rate: rate(aircraft.descent_rate()),
        approach_speed: speed(aircraft.approach_speed()),
        origin_elevation: from.elevation().to_meters().value(),
        destination_elevation: to.elevation().to_meters().value(),
        ..ProfileInputs::default()
    }
}

/// At a mapped destination, the first exit a landing at `approach_speed`
/// m/s can make: its node, position and the speed it is taken at.
fn exit_at(
    ground: LegGround<'_>,
    destination: Id<Airport>,
    runway: Option<RunwayDesignator>,
    approach_speed: f64,
) -> Option<(Id<Node>, LatLon, f64)> {
    let layout = ground.network.airport(destination)?;
    let (exit, speed) = landing_exit(layout, runway?, approach_speed)?;
    Some((
        exit.node(),
        layout.nodes()[exit.node().as_usize()].position(),
        speed,
    ))
}

/// The taxi-out of a leg.
struct TaxiOut {
    taxi: Polyline,
    lineup: Polyline,
    nodes: Option<(Id<Node>, Id<Node>)>,
    push_points: Vec<LatLon>,
    queue_points: Vec<LatLon>,
    roll_start: LatLon,
    stand_heading: Option<Degrees>,
    pushback: f64,
    routed: bool,
}

/// At a mapped origin: from the stand along taxiways to the entry nearest the
/// threshold (plan M4 D71, D72); without a route straight from the stand, or
/// from the airport point (D47).
fn taxi_out(
    ground: LegGround<'_>,
    origin: Id<Airport>,
    runway: Option<RunwayDesignator>,
    wingspan: Meters,
    airport_point: LatLon,
    threshold: LatLon,
) -> Result<TaxiOut> {
    let layout = ground.network.airport(origin);
    let stand = layout.zip(ground.origin_stand);
    if let Some(((layout, stand), runway)) = stand.zip(runway)
        && let Some(plan) = departure(layout, stand, (runway, threshold), wingspan)
    {
        return Ok(TaxiOut {
            taxi: plan.taxi,
            lineup: plan.lineup,
            nodes: Some((plan.hold, plan.joint)),
            push_points: plan.push_points,
            queue_points: plan.queue_points,
            roll_start: plan.entry,
            stand_heading: Some(plan.stand_heading),
            pushback: plan.pushback,
            routed: true,
        });
    }
    let parked = stand.and_then(|(layout, id)| {
        let stand = layout.stands().get(id.as_usize())?;
        Some((
            layout.nodes()[stand.node().as_usize()].position(),
            stand.heading(),
        ))
    });
    let start = parked.map_or(airport_point, |(point, _)| point);
    Ok(TaxiOut {
        taxi: Polyline::new(&[start, threshold])?,
        lineup: Polyline::new(&[threshold])?,
        nodes: None,
        push_points: Vec::new(),
        queue_points: Vec::new(),
        roll_start: threshold,
        stand_heading: parked.map(|(_, heading)| heading),
        pushback: 0.0,
        routed: false,
    })
}

/// How badly runway end `end` suits a departure on course `direct`
/// towards `ahead` (lower is better). Parallel runways align equally: the
/// one on the side of the course wins, by a degree per kilometre `ahead`
/// lies off its extended centreline (staggered thresholds aside).
fn departure_score(end: &RunwayEnd, direct: Degrees, ahead: LatLon) -> f64 {
    let threshold = end.threshold();
    let off = (threshold.initial_bearing_to(ahead).value() - end.true_heading().value())
        .to_radians()
        .sin()
        .abs()
        * threshold.distance_to(ahead).value();
    angle_between(end.true_heading(), direct) + off / 1000.0
}

/// Plans `aircraft` flying from `origin` to `destination` with the runways
/// in use at both ends and simple procedures between them.
pub(crate) fn plan_leg(
    world: &World,
    traffic: &TrafficTargets,
    ground: LegGround<'_>,
    aircraft: &AircraftType,
    origin: Id<Airport>,
    destination: Id<Airport>,
) -> Result<Leg> {
    let from = world.airport(origin).context("origin out of range")?;
    let to = world
        .airport(destination)
        .context("destination out of range")?;
    let direct = from.location().initial_bearing_to(to.location());
    let ahead = from
        .location()
        .destination(direct, Meters::from_raw(SIDE_PROBE));
    // Depart from the mode's runway closest to the course; land on the one
    // whose final is nearest the origin.
    let (departure_point, departure_course, departure_runway) = runway_end(
        world,
        traffic,
        origin,
        aircraft,
        Use::Departure,
        |end| departure_score(end, direct, ahead),
        direct,
    )?;
    let (arrival, arrival_course, arrival_runway) = runway_end(
        world,
        traffic,
        destination,
        aircraft,
        Use::Arrival,
        |end| {
            end.threshold()
                .destination(reverse(end.true_heading()), Meters::from_raw(FINAL_LENGTH))
                .distance_to(from.location())
                .value()
        },
        direct,
    )?;

    let mut inputs = profile_inputs(aircraft, from, to);
    let wingspan = aircraft.wingspan();
    let out = taxi_out(
        ground,
        origin,
        departure_runway.map(|(_, designator)| designator),
        wingspan,
        from.location(),
        departure_point,
    )?;
    inputs.pushback_length = out.pushback;
    inputs.lineup_length = out.lineup.length().value();
    let roll_start = out.roll_start;
    let planned_exit = exit_at(
        ground,
        destination,
        arrival_runway.map(|(_, designator)| designator),
        inputs.approach_speed,
    );
    // Roll lengths and climb speed depend only on speeds, so a provisional
    // profile gives them.
    let provisional = Profile::new(&inputs);
    let liftoff = roll_start.destination(
        departure_course,
        Meters::from_raw(provisional.takeoff_length()),
    );
    let (roll_end, exit) = match planned_exit {
        Some((node, point, exit_speed)) => {
            inputs.exit_speed = exit_speed;
            (point, Some(node))
        }
        None => (roll_out(arrival, arrival_course, &provisional), None),
    };
    let turn_point = departure_turn_point(
        liftoff,
        departure_course,
        climb_out_distance(inputs.climb_rate, provisional.climb_entry_speed()),
    );
    let mut waypoints = vec![liftoff, turn_point];
    waypoints.extend(arrival_points(turn_point, arrival, arrival_course));
    let fix = waypoints[waypoints.len() - 1];
    let enroute =
        Route::new(&waypoints, Meters::from_raw(TURN_RADIUS)).context("cannot route the leg")?;
    let path = Path {
        taxi_out: out.taxi,
        lineup: out.lineup,
        takeoff: Segment::new(roll_start, liftoff)?,
        enroute,
        approach: Segment::new(fix, arrival)?,
        landing: Segment::new(arrival, roll_end)?,
        taxi_in: Polyline::new(&[roll_end, to.location()])?,
        departure_runway: departure_runway.map(|(index, _)| index),
        arrival_runway: arrival_runway.map(|(index, _)| index),
        routed_out: out.routed,
        stand_out: out.stand_heading,
        stand_in: None,
        exit,
        hold: out.nodes.map(|(hold, _)| hold),
        joint: out.nodes.map(|(_, joint)| joint),
        push_points: out.push_points,
        queue_points: out.queue_points,
    };
    measure(&path, &mut inputs);
    Ok(Leg {
        path,
        profile: Profile::new(&inputs),
    })
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, bail};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        ops::{Phase, motion::Motion},
        test_data::{Data, data},
        world::AirportKind,
    };

    static NO_GROUND: GroundNetwork = GroundNetwork::empty();

    /// Legs without ground layouts: straight-line taxiing (D47).
    fn unmapped() -> LegGround<'static> {
        LegGround {
            network: &NO_GROUND,
            origin_stand: None,
        }
    }

    fn airport(data: &Data, ident: &str) -> Result<Id<Airport>> {
        data.world
            .airport_by_ident(ident.parse()?)
            .context("airport")
    }

    fn a320(data: &Data) -> Result<&AircraftType> {
        let id = data.aircraft.by_code("A320".parse()?).context("A320")?;
        data.aircraft.get(id).context("A320")
    }

    /// Flies the whole leg, checking that positions never jump.
    fn fly(leg: &Leg, dt: f64) -> Result<LatLon> {
        let mut motion = Motion::start(&leg.profile, 480.0, 300.0);
        let (mut last, _) = leg.path.locate(&motion);
        for _ in 0..200_000 {
            motion.advance_free(&leg.profile, dt);
            let (here, heading) = leg.path.locate(&motion);
            let jump = last.distance_to(here).value();
            if jump > motion.speed().max(80.0) * dt * 1.5 + 5.0 {
                bail!("jumped {jump:.0} m in {:?}", motion.phase());
            }
            if !(0.0..360.0).contains(&heading.value()) {
                bail!("heading {heading}");
            }
            last = here;
            if motion.phase() == Phase::Arrived {
                return Ok(here);
            }
        }
        bail!("never arrived")
    }

    #[test]
    fn test_plan_leg_frankfurt_to_munich() -> Result<()> {
        let data = data()?;
        let (frankfurt, munich) = (airport(&data, "EDDF")?, airport(&data, "EDDM")?);
        let leg = plan_leg(
            &data.world,
            &data.traffic,
            unmapped(),
            a320(&data)?,
            frankfurt,
            munich,
        )?;
        let distance = data
            .world
            .airport(frankfurt)
            .context("FRA")?
            .location()
            .distance_to(data.world.airport(munich).context("MUC")?.location());
        let enroute = leg.path.enroute.length().value();
        assert!((enroute - distance.value()).abs() < 60_000.0, "{enroute}");
        // South-east bound in west flow: runway 18 is the departure runway
        // closest to the course (D55).
        let course = leg.path.takeoff.bearing_at(Meters::default()).value();
        assert!((course - 180.0).abs() < 5.0, "take-off course {course}");
        // Straight ahead on the runway heading through the climb-out.
        for metres in [500.0, 2_000.0, 6_000.0] {
            let along = leg.path.enroute.bearing_at(Meters::new(metres)?).value();
            assert!((along - course).abs() < 2.0, "{along} at {metres} m");
        }
        assert!((leg.path.approach.length().value() - FINAL_LENGTH).abs() < 1.0);
        let gate = fly(&leg, 1.0)?;
        let munich_point = data.world.airport(munich).context("MUC")?.location();
        assert!(gate.distance_to(munich_point).value() < 1.0);
        Ok(())
    }

    #[test]
    fn test_plan_leg_without_runways_uses_the_airport_point() -> Result<()> {
        let world = World::from_json(
            r#"{"format_version": 2, "runways": [], "airports": [
                {"ident": "DE-0001", "name": "A", "location": {"lat": 50.0, "lon": 8.0},
                 "elevation": 300.0, "kind": "airfield", "country": "DE"},
                {"ident": "DE-0002", "name": "B", "location": {"lat": 52.0, "lon": 13.0},
                 "elevation": 100.0, "kind": "airfield", "country": "DE"}]}"#,
        )?;
        let data = data()?;
        let leg = plan_leg(
            &world,
            &data.traffic,
            unmapped(),
            a320(&data)?,
            Id::new(0),
            Id::new(1),
        )?;
        let start = leg.path.takeoff.point_at(Meters::default());
        assert!(start.distance_to(LatLon::new(50.0, 8.0)?).value() < 1.0);
        fly(&leg, 1.0)?;
        Ok(())
    }

    #[test]
    fn test_parallel_departure_runways_split_by_side() -> Result<()> {
        let data = data()?;
        let munich = airport(&data, "EDDM")?;
        let runway_of = |destination: &str| -> Result<String> {
            let leg = plan_leg(
                &data.world,
                &data.traffic,
                unmapped(),
                a320(&data)?,
                munich,
                airport(&data, destination)?,
            )?;
            let index = leg.path.departure_runway.context("a runway")?;
            Ok(data.world.runways_of(munich)[index].name())
        };
        // Hamburg is north, Zurich south-west: different parallel runways.
        assert_ne!(runway_of("EDDH")?, runway_of("LSZH")?);
        // Stuttgart lies just north of west: the north runway too, although
        // the south runway's threshold lies farther west.
        assert_eq!(runway_of("EDDS")?, runway_of("EDDH")?);
        Ok(())
    }

    #[test]
    fn test_runway_modes_pick_runways_by_direction() -> Result<()> {
        let data = data()?;
        let (frankfurt, munich) = (airport(&data, "EDDF")?, airport(&data, "EDDM")?);
        let amsterdam = airport(&data, "EHAM")?;
        let north_west = plan_leg(
            &data.world,
            &data.traffic,
            unmapped(),
            a320(&data)?,
            frankfurt,
            amsterdam,
        )?;
        let course = north_west
            .path
            .takeoff
            .bearing_at(Meters::default())
            .value();
        assert!(
            (course - 249.0).abs() < 5.0,
            "25C to the north-west, not {course}"
        );
        // Arriving from Munich, the south-eastern final of 25L is closer than
        // that of 25R.
        let inbound = plan_leg(
            &data.world,
            &data.traffic,
            unmapped(),
            a320(&data)?,
            munich,
            frankfurt,
        )?;
        let threshold = inbound.path.landing.point_at(Meters::default());
        let runways = data.world.runways_of(frankfurt);
        let landed_on = runways
            .iter()
            .flat_map(|runway| runway.ends().iter())
            .min_by(|a, b| {
                a.threshold()
                    .distance_to(threshold)
                    .value()
                    .total_cmp(&b.threshold().distance_to(threshold).value())
            })
            .context("an end")?;
        assert_eq!(landed_on.designator().to_string(), "25L");
        Ok(())
    }

    static GROUND: std::sync::LazyLock<Result<GroundNetwork, String>> =
        std::sync::LazyLock::new(|| {
            let world = data().map_err(|e| format!("{e:#}"))?.world;
            GroundNetwork::from_json(
                include_str!("../../../../../assets/ground/germany.ground.json"),
                &world,
            )
            .map_err(|e| format!("{e:#}"))
        });

    #[test]
    fn test_mapped_legs_taxi_from_the_stand_and_vacate_at_an_exit() -> Result<()> {
        let data = data()?;
        let network = GROUND.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let (frankfurt, munich) = (airport(&data, "EDDF")?, airport(&data, "EDDM")?);
        let layout = network.airport(frankfurt).context("FRA layout")?;
        let stand = layout
            .stands()
            .iter()
            .position(|s| s.name().as_str() == "A14")
            .context("stand A14")?;
        let leg = plan_leg(
            &data.world,
            &data.traffic,
            LegGround {
                network,
                origin_stand: Some(Id::from_usize(stand)?),
            },
            a320(&data)?,
            frankfurt,
            munich,
        )?;
        let at_stand = layout.nodes()[layout.stands()[stand].node().as_usize()].position();
        let start = leg.path.taxi_out.point_at(Meters::default());
        assert!(start.distance_to(at_stand).value() < 0.5);
        assert!(leg.path.routed_out());
        assert!(
            leg.profile.pushback_length() > 10.0,
            "A14 is a nose-in stand"
        );
        // Taxied to the holding position of 18, lined up, then the roll.
        let end = leg.path.taxi_out.point_at(leg.path.taxi_out.length());
        let lined = leg.path.lineup.point_at(Meters::default());
        assert!(end.distance_to(lined).value() < 0.5);
        let onto = leg.path.lineup.point_at(leg.path.lineup.length());
        let roll = leg.path.takeoff.point_at(Meters::default());
        assert!(onto.distance_to(roll).value() < 0.5);
        assert!(
            leg.profile.lineup_length() > 20.0,
            "{}",
            leg.profile.lineup_length()
        );
        assert!(leg.path.taxi_out.length().value() > 1_000.0);
        // At Munich the roll ends at an exit node.
        let exit = leg.path.exit().context("an exit at MUC")?;
        let muc = network.airport(munich).context("MUC layout")?;
        let exit_point = muc.nodes()[exit.as_usize()].position();
        assert!(leg.path.exit_point().distance_to(exit_point).value() < 0.5);
        let rolled = leg.profile.landing_length();
        assert!((900.0..=3_500.0).contains(&rolled), "{rolled}");
        // The whole leg flies without jumps, pushback included.
        let gate = fly(&leg, 1.0)?;
        let muc_point = data.world.airport(munich).context("MUC")?.location();
        assert!(
            gate.distance_to(muc_point).value() < 1.0,
            "provisional taxi-in"
        );
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_planned_legs_are_continuous(first in 0_usize..1000, second in 0_usize..1000) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let data = DATA.as_ref().map_err(|e| TestCaseError::fail(e.clone()))?;
            let served: Vec<Id<Airport>> = data
                .world
                .airports()
                .filter(|(_, a)| matches!(a.kind(), AirportKind::Major | AirportKind::Edge))
                .map(|(id, _)| id)
                .collect();
            let origin = served[first % served.len()];
            let destination = served[second % served.len()];
            prop_assume!(origin != destination);
            let leg = plan_leg(&data.world, &data.traffic, unmapped(), a320(data).map_err(fail)?, origin, destination)
                .map_err(fail)?;
            let gate = fly(&leg, 10.0).map_err(fail)?;
            let wanted = data.world.airport(destination).map(Airport::location);
            prop_assert!(wanted.is_some_and(|point| gate.distance_to(point).value() < 1.0));
        }
    }

    static DATA: std::sync::LazyLock<Result<Data, String>> =
        std::sync::LazyLock::new(|| data().map_err(|e| format!("{e:#}")));
}
