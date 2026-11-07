//! A leg's geometry (D31, D36, D47, D59; plan M4): stand, taxi routes,
//! runway entry, lift-off point, procedure route to the final approach fix,
//! final, landing roll to the exit, joined end to end.

mod ground;
mod plan;

pub(crate) use self::{
    ground::arrival as arrival_route,
    plan::{LegGround, plan_leg},
};
use super::{Phase, motion::Motion};
use crate::{
    Id, LatLon,
    geo::{Polyline, Route, Segment},
    ground::Node,
    units::{Degrees, Meters},
};

/// Where a leg's parts lie.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Path {
    /// From the stand (or the airport point) to the runway holding
    /// position (or entry); starts with the pushback where there is one.
    taxi_out: Polyline,
    /// From the holding position onto the runway; a single point without
    /// one.
    lineup: Polyline,
    takeoff: Segment,
    enroute: Route,
    approach: Segment,
    landing: Segment,
    /// From the runway exit to the stand (or the airport point).
    taxi_in: Polyline,
    /// Index of the runway used at the origin and at the destination.
    departure_runway: Option<usize>,
    arrival_runway: Option<usize>,
    /// Whether the taxi-out follows taxiways (else a straight line).
    routed_out: bool,
    /// Heading on the stands at both ends, where known.
    stand_out: Option<Degrees>,
    stand_in: Option<Degrees>,
    /// The exit node at a mapped destination, from which the taxi-in is
    /// routed once a stand is assigned.
    exit: Option<Id<Node>>,
    /// At a mapped origin: the holding position and where the pushback
    /// ends (plan M4 PR 3).
    hold: Option<Id<Node>>,
    joint: Option<Id<Node>>,
    /// From the holding position node on to where the line-up starts.
    queue_points: Vec<LatLon>,
}

/// A leg ready to fly: geometry and profile.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Leg {
    pub(crate) path: Path,
    pub(crate) profile: super::motion::Profile,
}

fn reverse(course: Degrees) -> Degrees {
    Degrees::from_raw(course.value() + 180.0).normalized()
}

impl Path {
    /// The fix and inbound course of the hold `motion` is flying, if any.
    pub(crate) fn hold_fix(&self, motion: &Motion) -> Option<(LatLon, Degrees)> {
        (motion.phase() == Phase::Holding).then(|| {
            let along = Meters::from_raw(motion.along());
            (self.enroute.point_at(along), self.enroute.bearing_at(along))
        })
    }

    /// Indices of the runways used at the origin and the destination.
    pub(crate) const fn runways(&self) -> (Option<usize>, Option<usize>) {
        (self.departure_runway, self.arrival_runway)
    }

    /// Whether the taxi-out follows a route from a stand.
    pub(crate) const fn routed_out(&self) -> bool {
        self.routed_out
    }

    /// The holding position and pushback end of a routed departure.
    pub(crate) fn departure_nodes(&self) -> Option<(Id<Node>, Id<Node>)> {
        self.hold.zip(self.joint)
    }

    /// From the holding position node on to where the line-up starts.
    pub(crate) fn queue_points(&self) -> &[LatLon] {
        &self.queue_points
    }

    /// Taxies out along `route` (pushback included); returns its length, m.
    pub(crate) fn set_taxi_out(&mut self, route: Polyline) -> f64 {
        self.taxi_out = route;
        self.taxi_out.length().value()
    }

    /// The exit node at a mapped destination.
    pub(crate) const fn exit(&self) -> Option<Id<Node>> {
        self.exit
    }

    /// Where the landing roll ends.
    pub(crate) fn exit_point(&self) -> LatLon {
        self.landing.point_at(self.landing.length())
    }

    /// Ends the landing roll at `exit`, at `point`, instead (a later exit
    /// chosen at touchdown); returns the new roll length, m.
    pub(crate) fn set_exit(&mut self, exit: Id<Node>, point: LatLon) -> Option<f64> {
        let landing = Segment::new(self.landing.point_at(Meters::default()), point).ok()?;
        self.landing = landing;
        self.exit = Some(exit);
        self.taxi_in = Polyline::new(&[point, point]).ok()?;
        Some(self.landing.length().value().max(1.0))
    }

    /// Taxies in along `route` to a stand facing `heading`; returns the new
    /// taxi-in length, m.
    pub(crate) fn set_taxi_in(&mut self, route: Polyline, heading: Degrees) -> f64 {
        self.taxi_in = route;
        self.stand_in = Some(heading);
        self.taxi_in.length().value()
    }

    /// Position and true course of an aircraft in `motion` on this path.
    pub(crate) fn locate(&self, motion: &Motion) -> (LatLon, Degrees) {
        let along = Meters::from_raw(motion.along());
        match motion.phase() {
            Phase::Boarding => {
                let (point, course) = on(&self.taxi_out, Meters::default());
                (point, self.stand_out.unwrap_or(course))
            }
            Phase::Pushback => {
                let (point, course) = on(&self.taxi_out, along);
                (point, reverse(course))
            }
            Phase::TaxiOut => on(&self.taxi_out, along),
            Phase::Queue => on(&self.taxi_out, self.taxi_out.length()),
            Phase::LineUp => on(&self.lineup, along),
            Phase::Holding => super::hold::racetrack(
                self.enroute.point_at(along),
                self.enroute.bearing_at(along),
                motion.held_for(),
            ),
            Phase::TakeoffRoll => at(&self.takeoff, along),
            Phase::Climb | Phase::Cruise | Phase::Descent => {
                (self.enroute.point_at(along), self.enroute.bearing_at(along))
            }
            Phase::Approach => at(&self.approach, along),
            Phase::Landing => at(&self.landing, along),
            Phase::TaxiIn => on(&self.taxi_in, along),
            Phase::Arrived => {
                let (point, course) = on(&self.taxi_in, self.taxi_in.length());
                (point, self.stand_in.unwrap_or(course))
            }
        }
    }
}

fn at(segment: &Segment, along: Meters) -> (LatLon, Degrees) {
    (segment.point_at(along), segment.bearing_at(along))
}

fn on(line: &Polyline, along: Meters) -> (LatLon, Degrees) {
    (line.point_at(along), line.bearing_at(along))
}
