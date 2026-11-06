//! The operations controller (D48): every tail flies its weekly rotation,
//! leg after leg, with delays carried along the rotation.

use anyhow::{Context, Result};

mod start;

use start::Rotation;

use super::{
    Phase, StartMode,
    motion::{Event, Motion},
    path::{Leg, plan_leg},
    tail::{draw_delay, draw_taxi_in, draw_taxi_out, next_leg, off_block},
    view::{Callsign, FlightView},
};
use crate::{
    Id, RngSeed, SimDuration, SimRng, SimTime, StreamId, TICK,
    airline::AirlineIcao,
    scenario::Scenario,
    schedule::{Schedule, ScheduledFlight, Tail, min_turnaround},
    time::{LocalDate, Zone},
    units::{Meters, MetersPerSecond},
    world::{Airport, AirportKind},
};

/// Boarding starts this long before off-block.
const BOARDING: SimDuration = SimDuration::from_minutes(30);
/// Where one tail is in its rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TailOps {
    /// Index of the current leg in the tail's rotation.
    leg: usize,
    /// Local Monday of the week the current leg belongs to.
    week: LocalDate,
    scheduled: SimTime,
    off_block: SimTime,
    boarding_from: SimTime,
    /// `None` while parked before boarding.
    motion: Option<Motion>,
}

/// Every tail flying its rotation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Operations {
    time: SimTime,
    schedule: Schedule,
    tails: Vec<TailOps>,
    /// The planned current leg of each tail, while it boards or flies.
    legs: Vec<Option<Leg>>,
    zones: Vec<Zone>,
    turnarounds: Vec<SimDuration>,
    callsigns: Vec<AirlineIcao>,
    major: Vec<bool>,
    rng: SimRng,
    movements: Vec<u32>,
    airborne: usize,
    flights_flown: u64,
}

fn add(time: SimTime, duration: SimDuration) -> Result<SimTime> {
    time.checked_add(duration)
        .with_context(|| format!("{time} + {duration} is out of range"))
}

fn seconds_between(from: SimTime, to: SimTime) -> f64 {
    to.duration_since(from).as_secs_f64()
}

impl Operations {
    /// Operations for `schedule` from `start`, placed according to `mode`.
    pub(crate) fn new(
        scenario: &Scenario,
        schedule: Schedule,
        start: SimTime,
        seed: RngSeed,
        mode: StartMode,
    ) -> Result<Self> {
        let traffic = scenario
            .traffic()
            .context("operations need traffic inputs")?;
        let world = scenario.world();
        let utc = Zone::new("UTC")?;
        let mut zones = Vec::with_capacity(schedule.tails().len());
        let mut turnarounds = Vec::with_capacity(schedule.tails().len());
        for (_, tail) in schedule.tails() {
            let base = world
                .airport(tail.base())
                .context("tail base out of range")?;
            zones.push(traffic.targets().zone(base.ident()).unwrap_or(&utc).clone());
            let aircraft = traffic
                .aircraft()
                .get(tail.aircraft())
                .context("tail type")?;
            turnarounds.push(min_turnaround(aircraft));
        }
        let tail_count = schedule.tails().len();
        let mut operations = Self {
            time: start,
            tails: Vec::with_capacity(tail_count),
            legs: vec![None; tail_count],
            zones,
            turnarounds,
            callsigns: traffic
                .airlines()
                .iter()
                .map(crate::airline::Airline::icao)
                .collect(),
            major: world
                .airports()
                .map(|(_, airport)| airport.kind() == AirportKind::Major)
                .collect(),
            rng: seed.stream(StreamId::Operations),
            movements: vec![0; world.airport_count()],
            airborne: 0,
            flights_flown: 0,
            schedule,
        };
        for index in 0..operations.zones.len() {
            let tail = operations.place_tail(scenario, index, mode)?;
            operations.tails.push(tail);
        }
        operations.airborne = operations
            .tails
            .iter()
            .filter(|tail| tail.motion.is_some_and(|m| m.phase().is_airborne()))
            .count();
        Ok(operations)
    }

    fn rotation(&self, index: usize) -> Result<Rotation<'_>> {
        Ok(Rotation {
            flights: self.schedule.flights_of(Id::from_usize(index)?),
            zone: self.zones.get(index).context("tail out of range")?,
            turnaround: self.turnarounds[index],
        })
    }

    fn flight(&self, index: usize, leg: usize) -> Result<&ScheduledFlight> {
        self.schedule
            .flights_of(Id::from_usize(index)?)
            .get(leg)
            .context("leg out of range")
    }

    fn plan(&self, scenario: &Scenario, index: usize, leg: usize) -> Result<Leg> {
        let flight = self.flight(index, leg)?;
        let traffic = scenario.traffic().context("traffic")?;
        let aircraft = traffic.aircraft().get(flight.aircraft()).context("type")?;
        plan_leg(
            scenario.world(),
            aircraft,
            flight.origin(),
            flight.destination(),
        )
        .with_context(|| format!("cannot plan leg {leg} of tail {index}"))
    }

    fn is_major(&self, airport: Id<Airport>) -> bool {
        self.major.get(airport.as_usize()).copied().unwrap_or(false)
    }

    /// Advances every tail by one tick.
    pub(crate) fn step(&mut self, scenario: &Scenario) -> Result<()> {
        let now = self.time;
        for index in 0..self.tails.len() {
            self.step_tail(scenario, index, now)
                .with_context(|| format!("tail {index} at {now}"))?;
        }
        self.time = add(now, TICK)?;
        Ok(())
    }

    fn step_tail(&mut self, scenario: &Scenario, index: usize, now: SimTime) -> Result<()> {
        let mut tail = self.tails[index];
        match tail.motion {
            None => {
                if now >= tail.boarding_from {
                    let planned = self.plan(scenario, index, tail.leg)?;
                    tail.motion = Some(Motion::boarding(&planned.profile));
                    self.legs[index] = Some(planned);
                }
            }
            Some(motion) if motion.phase() == Phase::Boarding => {
                if now >= tail.off_block {
                    let flight = *self.flight(index, tail.leg)?;
                    let (origin, destination) = (
                        self.is_major(flight.origin()),
                        self.is_major(flight.destination()),
                    );
                    let out = draw_taxi_out(&mut self.rng, origin)?;
                    let back = draw_taxi_in(&mut self.rng, destination)?;
                    let planned = self.legs[index]
                        .as_ref()
                        .context("boarding without a leg")?;
                    tail.off_block = now;
                    tail.motion = Some(Motion::start(&planned.profile, out, back));
                }
            }
            Some(mut motion) => {
                let planned = self.legs[index].as_ref().context("flying without a leg")?;
                let event = motion.advance(&planned.profile, TICK.as_secs_f64());
                tail.motion = Some(motion);
                let flight = *self.flight(index, tail.leg)?;
                match event {
                    Some(Event::Takeoff) => {
                        self.airborne += 1;
                        self.count(flight.origin());
                    }
                    Some(Event::Landing) => {
                        self.airborne = self.airborne.saturating_sub(1);
                        self.count(flight.destination());
                    }
                    Some(Event::Arrived) => {
                        self.flights_flown += 1;
                        self.legs[index] = None;
                        tail = self.next_tail_leg(index, &tail, add(now, TICK)?)?;
                    }
                    None => {}
                }
            }
        }
        self.tails[index] = tail;
        Ok(())
    }

    fn count(&mut self, airport: Id<Airport>) {
        if let Some(count) = self.movements.get_mut(airport.as_usize()) {
            *count += 1;
        }
    }

    /// The tail's next leg after arriving at `arrived`.
    fn next_tail_leg(&mut self, index: usize, tail: &TailOps, arrived: SimTime) -> Result<TailOps> {
        let rotation = self.rotation(index)?;
        let (leg, week) = next_leg(tail.leg, rotation.flights.len(), tail.week)?;
        let scheduled = rotation.scheduled(leg, week)?;
        let ready = add(arrived, rotation.turnaround)?;
        let delay = draw_delay(&mut self.rng)?;
        let leaves = off_block(scheduled, delay, ready)?;
        Ok(TailOps {
            leg,
            week,
            scheduled,
            off_block: leaves,
            boarding_from: leaves.checked_sub(BOARDING)?.max(arrived),
            motion: None,
        })
    }

    /// The time the next step simulates.
    #[cfg(test)]
    pub(crate) const fn time(&self) -> SimTime {
        self.time
    }

    /// The schedule being flown.
    pub(crate) const fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Aircraft currently off the ground.
    pub(crate) const fn airborne(&self) -> usize {
        self.airborne
    }

    /// Take-offs plus landings at `airport` since the start.
    pub(crate) fn movements(&self, airport: Id<Airport>) -> u32 {
        self.movements.get(airport.as_usize()).copied().unwrap_or(0)
    }

    /// Legs completed (on blocks) since the start.
    pub(crate) const fn flights_flown(&self) -> u64 {
        self.flights_flown
    }

    /// Every boarding or moving flight.
    pub(crate) fn flights(&self) -> impl Iterator<Item = FlightView> + '_ {
        self.tails.iter().enumerate().filter_map(|(index, tail)| {
            let motion = tail.motion?;
            let planned = self.legs.get(index)?.as_ref()?;
            let flight = self.flight(index, tail.leg).ok()?;
            let (position, heading) = planned.path.locate(&motion);
            Some(FlightView {
                tail: Id::<Tail>::from_usize(index).ok()?,
                callsign: Callsign::new(
                    *self.callsigns.get(flight.airline().as_usize())?,
                    flight.number(),
                ),
                airline: flight.airline(),
                aircraft: flight.aircraft(),
                kind: flight.kind(),
                origin: flight.origin(),
                destination: flight.destination(),
                phase: motion.phase(),
                position,
                heading,
                altitude: Meters::from_raw(motion.altitude()).to_feet(),
                ground_speed: MetersPerSecond::from_raw(motion.speed()).to_knots(),
                scheduled_departure: tail.scheduled,
                scheduled_arrival: tail.scheduled.checked_add(flight.block()).ok()?,
                off_block: tail.off_block,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed,
        schedule::{ScheduleConfig, generate},
        test_data::data,
    };

    /// Wednesday 8 July 2026, 10:00 UTC: midday traffic.
    fn midday() -> Result<SimTime> {
        "2026-07-08T10:00:00Z".parse()
    }

    fn setup(mode: StartMode, seed: u64) -> Result<(Scenario, Operations)> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), midday()?))?;
        let operations = Operations::new(&scenario, schedule, midday()?, RngSeed::new(seed), mode)?;
        Ok((scenario, operations))
    }

    fn run(operations: &mut Operations, scenario: &Scenario, ticks: u32) -> Result<()> {
        for _ in 0..ticks {
            operations.step(scenario)?;
        }
        Ok(())
    }

    fn airborne_views(operations: &Operations) -> usize {
        operations
            .flights()
            .filter(|f| f.phase().is_airborne())
            .count()
    }

    #[test]
    fn test_warm_start_fills_the_sky() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 1)?;
        assert!(operations.airborne() > 300, "{}", operations.airborne());
        assert_eq!(airborne_views(&operations), operations.airborne());
        assert_eq!(operations.time(), midday()?);
        run(&mut operations, &scenario, 600)?;
        assert_eq!(airborne_views(&operations), operations.airborne());
        assert_eq!(
            operations.time(),
            midday()?.checked_add(SimDuration::from_minutes(10))?
        );
        Ok(())
    }

    #[test]
    fn test_cold_start_begins_on_the_ground() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Cold, 1)?;
        assert_eq!(operations.airborne(), 0);
        run(&mut operations, &scenario, 3_600)?;
        assert!(operations.airborne() > 100, "{}", operations.airborne());
        assert_eq!(airborne_views(&operations), operations.airborne());
        Ok(())
    }

    #[test]
    fn test_operations_are_deterministic() -> Result<()> {
        let (scenario, mut first) = setup(StartMode::Warm, 3)?;
        let (_, mut second) = setup(StartMode::Warm, 3)?;
        let (_, mut other) = setup(StartMode::Warm, 4)?;
        run(&mut first, &scenario, 900)?;
        run(&mut second, &scenario, 900)?;
        run(&mut other, &scenario, 900)?;
        assert_eq!(first, second);
        assert_ne!(first, other);
        Ok(())
    }

    #[test]
    fn test_movements_and_completed_flights_are_counted() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 2)?;
        run(&mut operations, &scenario, 1_800)?;
        let frankfurt = scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        // FRA handles about 1,200 movements a day: dozens per half hour.
        assert!(
            operations.movements(frankfurt) > 20,
            "{}",
            operations.movements(frankfurt)
        );
        assert!(operations.flights_flown() > 100);
        Ok(())
    }

    #[test]
    fn test_flight_views_are_plausible() -> Result<()> {
        let (scenario, mut operations) = setup(StartMode::Warm, 5)?;
        run(&mut operations, &scenario, 120)?;
        let traffic = scenario.traffic().context("traffic")?;
        let mut seen = 0;
        for flight in operations.flights() {
            seen += 1;
            let origin = scenario
                .world()
                .airport(flight.origin())
                .context("origin")?;
            let airline = traffic
                .airlines()
                .get(flight.airline())
                .context("airline")?;
            assert!(
                flight
                    .callsign()
                    .to_string()
                    .starts_with(&airline.icao().to_string())
            );
            let latitude = flight.position().latitude().value();
            assert!((25.0..72.0).contains(&latitude), "{latitude}");
            assert!(flight.scheduled_arrival() > flight.scheduled_departure());
            assert!(flight.off_block() >= flight.scheduled_departure());
            match flight.phase() {
                Phase::Boarding => {
                    assert!(flight.position().distance_to(origin.location()).value() < 1.0);
                    assert!(flight.ground_speed().value().abs() < 1e-9);
                }
                Phase::Cruise => assert!(flight.altitude().value() > 10_000.0),
                phase if phase.is_airborne() => assert!(flight.ground_speed().value() > 100.0),
                _ => {}
            }
        }
        assert!(seen > operations.airborne());
        Ok(())
    }
}
