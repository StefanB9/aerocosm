//! A leg's geometry (D31, D36, D47): gate, runway ends, lift-off point, final
//! approach fix and landing roll as segments joined end to end.

use anyhow::{Context, Result};

use super::{
    Phase,
    motion::{FINAL_LENGTH, Motion, Profile, ProfileInputs},
    runway::{PREVAILING_WIND, runway_in_use},
};
use crate::{
    AircraftType, Id, LatLon,
    geo::Segment,
    units::{Degrees, Meters},
    world::{Airport, World},
};

/// Runways down to this fraction of the type's take-off field length are
/// used, as in schedule generation.
const RUNWAY_MARGIN: f64 = 0.75;

/// Where a leg's segments lie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Path {
    taxi_out: Segment,
    takeoff: Segment,
    enroute: Segment,
    approach: Segment,
    landing: Segment,
    taxi_in: Segment,
}

/// A leg ready to fly: geometry and profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Leg {
    pub(crate) path: Path,
    pub(crate) profile: Profile,
}

/// A runway threshold and the course along it; the airport point and
/// `fallback` course without runways.
fn runway_end(
    world: &World,
    airport: Id<Airport>,
    aircraft: &AircraftType,
    fallback: Degrees,
) -> Result<(LatLon, Degrees)> {
    let runways = world.runways_of(airport);
    let min_length = aircraft.min_runway_length() * RUNWAY_MARGIN;
    Ok(match runway_in_use(runways, PREVAILING_WIND, min_length) {
        Some(choice) => {
            let end = &runways[choice.runway].ends()[choice.end];
            (end.threshold(), end.true_heading())
        }
        None => (
            world
                .airport(airport)
                .context("airport id out of range")?
                .location(),
            fallback,
        ),
    })
}

fn reverse(course: Degrees) -> Degrees {
    Degrees::from_raw(course.value() + 180.0).normalized()
}

/// Plans `aircraft` flying from `origin` to `destination` with the runways
/// in use at both ends.
pub(crate) fn plan_leg(
    world: &World,
    aircraft: &AircraftType,
    origin: Id<Airport>,
    destination: Id<Airport>,
) -> Result<Leg> {
    let from = world.airport(origin).context("origin out of range")?;
    let to = world
        .airport(destination)
        .context("destination out of range")?;
    let direct = from.location().initial_bearing_to(to.location());
    let (departure, departure_course) = runway_end(world, origin, aircraft, direct)?;
    let (arrival, arrival_course) = runway_end(world, destination, aircraft, direct)?;
    let fix = arrival.destination(reverse(arrival_course), Meters::from_raw(FINAL_LENGTH));

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
    // Roll lengths depend only on speeds, so a provisional profile gives them.
    let rolls = Profile::new(&inputs);
    let liftoff = departure.destination(departure_course, Meters::from_raw(rolls.takeoff_length()));
    let roll_end = arrival.destination(arrival_course, Meters::from_raw(rolls.landing_length()));
    let path = Path {
        taxi_out: Segment::new(from.location(), departure)?,
        takeoff: Segment::new(departure, liftoff)?,
        enroute: Segment::new(liftoff, fix).context("cannot route the leg")?,
        approach: Segment::new(fix, arrival)?,
        landing: Segment::new(arrival, roll_end)?,
        taxi_in: Segment::new(roll_end, to.location())?,
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
    /// Position and true course of an aircraft in `motion` on this path.
    pub(crate) fn locate(&self, motion: &Motion) -> (LatLon, Degrees) {
        let along = Meters::from_raw(motion.along());
        let segment = match motion.phase() {
            Phase::Boarding => return at(&self.taxi_out, Meters::default()),
            Phase::TaxiOut => &self.taxi_out,
            Phase::TakeoffRoll => &self.takeoff,
            Phase::Climb | Phase::Cruise | Phase::Descent => &self.enroute,
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
            motion.advance(&leg.profile, dt);
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
        let leg = plan_leg(&data.world, a320(&data)?, frankfurt, munich)?;
        let distance = data
            .world
            .airport(frankfurt)
            .context("FRA")?
            .location()
            .distance_to(data.world.airport(munich).context("MUC")?.location());
        let enroute = leg.path.enroute.length().value();
        assert!((enroute - distance.value()).abs() < 40_000.0, "{enroute}");
        // Departing into the 250° wind.
        let course = leg.path.takeoff.bearing_at(Meters::default()).value();
        assert!((course - 250.0).abs() < 25.0, "take-off course {course}");
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
        let leg = plan_leg(&world, a320(&data)?, Id::new(0), Id::new(1))?;
        let start = leg.path.takeoff.point_at(Meters::default());
        assert!(start.distance_to(LatLon::new(50.0, 8.0)?).value() < 1.0);
        fly(&leg, 1.0)?;
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
            let leg = plan_leg(&data.world, a320(data).map_err(fail)?, origin, destination)
                .map_err(fail)?;
            let gate = fly(&leg, 10.0).map_err(fail)?;
            let wanted = data.world.airport(destination).map(Airport::location);
            prop_assert!(wanted.is_some_and(|point| gate.distance_to(point).value() < 1.0));
        }
    }

    static DATA: std::sync::LazyLock<Result<Data, String>> =
        std::sync::LazyLock::new(|| data().map_err(|e| format!("{e:#}")));
}
