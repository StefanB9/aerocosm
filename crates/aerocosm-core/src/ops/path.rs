//! A leg's geometry (D31, D36, D47, D59): gate, runway ends, lift-off point,
//! procedure route to the final approach fix, final and landing roll, joined
//! end to end.

use anyhow::{Context, Result};

use super::{
    Phase,
    motion::{FINAL_LENGTH, Motion, Profile, ProfileInputs},
    procedure::{arrival_points, climb_out_distance, departure_turn_point},
    runway::{PREVAILING_WIND, RunwayChoice, mode_runway, runway_in_use},
};
use crate::{
    AircraftType, Id, LatLon,
    geo::{Route, Segment},
    traffic::{TrafficTargets, angle_between, mode_for_wind},
    units::{Degrees, Meters},
    world::{Airport, RunwayEnd, World},
};

/// Runways down to this fraction of the type's take-off field length are
/// used, as in schedule generation.
const RUNWAY_MARGIN: f64 = 0.75;
/// Turn radius of procedures: 210 kt at 25° of bank.
const TURN_RADIUS: f64 = 2_550.0;

/// Where a leg's parts lie.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Path {
    taxi_out: Segment,
    takeoff: Segment,
    enroute: Route,
    approach: Segment,
    landing: Segment,
    taxi_in: Segment,
    /// Index of the runway used at the origin and at the destination.
    departure_runway: Option<usize>,
    arrival_runway: Option<usize>,
}

/// A leg ready to fly: geometry and profile.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Leg {
    pub(crate) path: Path,
    pub(crate) profile: Profile,
}

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
) -> Result<(LatLon, Degrees, Option<usize>)> {
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
            (end.threshold(), end.true_heading(), Some(choice.runway))
        }
        None => (place.location(), fallback, None),
    })
}

fn reverse(course: Degrees) -> Degrees {
    Degrees::from_raw(course.value() + 180.0).normalized()
}

/// Plans `aircraft` flying from `origin` to `destination` with the runways
/// in use at both ends and simple procedures between them.
pub(crate) fn plan_leg(
    world: &World,
    traffic: &TrafficTargets,
    aircraft: &AircraftType,
    origin: Id<Airport>,
    destination: Id<Airport>,
) -> Result<Leg> {
    let from = world.airport(origin).context("origin out of range")?;
    let to = world
        .airport(destination)
        .context("destination out of range")?;
    let direct = from.location().initial_bearing_to(to.location());
    // Depart from the mode's runway closest to the course; land on the one
    // whose final is nearest the origin.
    let (departure, departure_course, departure_runway) = runway_end(
        world,
        traffic,
        origin,
        aircraft,
        Use::Departure,
        |end| angle_between(end.true_heading(), direct),
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

    let speed = |knots: crate::units::Knots| knots.to_meters_per_second().value();
    let rate = |rate: crate::units::FeetPerMinute| rate.to_meters_per_second().value();
    let mut inputs = ProfileInputs {
        cruise_speed: speed(aircraft.cruise_speed()),
        cruise_altitude: aircraft.cruise_altitude().to_meters().value(),
        climb_rate: rate(aircraft.climb_rate()),
        descent_rate: rate(aircraft.descent_rate()),
        approach_speed: speed(aircraft.approach_speed()),
        origin_elevation: from.elevation().to_meters().value(),
        destination_elevation: to.elevation().to_meters().value(),
        enroute_length: 0.0,
        taxi_out_length: 0.0,
        taxi_in_length: 0.0,
    };
    // Roll lengths and climb speed depend only on speeds, so a provisional
    // profile gives them.
    let provisional = Profile::new(&inputs);
    let liftoff = departure.destination(
        departure_course,
        Meters::from_raw(provisional.takeoff_length()),
    );
    let roll_end = arrival.destination(
        arrival_course,
        Meters::from_raw(provisional.landing_length()),
    );
    let turn_point = departure_turn_point(
        liftoff,
        departure_course,
        climb_out_distance(inputs.climb_rate, provisional.climb_entry_speed()),
    );
    let mut waypoints = vec![liftoff, turn_point];
    waypoints.extend(arrival_points(turn_point, arrival, arrival_course));
    let fix = waypoints[waypoints.len() - 1];
    let path = Path {
        taxi_out: Segment::new(from.location(), departure)?,
        takeoff: Segment::new(departure, liftoff)?,
        enroute: Route::new(&waypoints, Meters::from_raw(TURN_RADIUS))
            .context("cannot route the leg")?,
        approach: Segment::new(fix, arrival)?,
        landing: Segment::new(arrival, roll_end)?,
        taxi_in: Segment::new(roll_end, to.location())?,
        departure_runway,
        arrival_runway,
    };
    inputs.enroute_length = path.enroute.length().value();
    inputs.taxi_out_length = path.taxi_out.length().value();
    inputs.taxi_in_length = path.taxi_in.length().value();
    Ok(Leg {
        path,
        profile: Profile::new(&inputs),
    })
}

impl Path {
    /// Indices of the runways used at the origin and the destination.
    pub(crate) const fn runways(&self) -> (Option<usize>, Option<usize>) {
        (self.departure_runway, self.arrival_runway)
    }

    /// Position and true course of an aircraft in `motion` on this path.
    pub(crate) fn locate(&self, motion: &Motion) -> (LatLon, Degrees) {
        let along = Meters::from_raw(motion.along());
        let segment = match motion.phase() {
            Phase::Boarding => return at(&self.taxi_out, Meters::default()),
            Phase::TaxiOut => &self.taxi_out,
            Phase::Queue => return at(&self.taxi_out, self.taxi_out.length()),
            Phase::Holding => {
                // A racetrack at the hold point, inbound along the route.
                return super::hold::racetrack(
                    self.enroute.point_at(along),
                    self.enroute.bearing_at(along),
                    motion.held_for(),
                );
            }
            Phase::TakeoffRoll => &self.takeoff,
            Phase::Climb | Phase::Cruise | Phase::Descent => {
                return (self.enroute.point_at(along), self.enroute.bearing_at(along));
            }
            Phase::Approach => &self.approach,
            Phase::Landing => &self.landing,
            Phase::TaxiIn => &self.taxi_in,
            Phase::Arrived => return at(&self.taxi_in, self.taxi_in.length()),
        };
        at(segment, along)
    }
}

fn at(segment: &Segment, along: Meters) -> (LatLon, Degrees) {
    (segment.point_at(along), segment.bearing_at(along))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, bail};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        test_data::{Data, data},
        world::AirportKind,
    };

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
        let leg = plan_leg(&data.world, &data.traffic, a320(&data)?, frankfurt, munich)?;
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
        let leg = plan_leg(&world, &data.traffic, a320(&data)?, Id::new(0), Id::new(1))?;
        let start = leg.path.takeoff.point_at(Meters::default());
        assert!(start.distance_to(LatLon::new(50.0, 8.0)?).value() < 1.0);
        fly(&leg, 1.0)?;
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
        let inbound = plan_leg(&data.world, &data.traffic, a320(&data)?, munich, frankfurt)?;
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
            let leg = plan_leg(&data.world, &data.traffic, a320(data).map_err(fail)?, origin, destination)
                .map_err(fail)?;
            let gate = fly(&leg, 10.0).map_err(fail)?;
            let wanted = data.world.airport(destination).map(Airport::location);
            prop_assert!(wanted.is_some_and(|point| gate.distance_to(point).value() < 1.0));
        }
    }

    static DATA: std::sync::LazyLock<Result<Data, String>> =
        std::sync::LazyLock::new(|| data().map_err(|e| format!("{e:#}")));
}
