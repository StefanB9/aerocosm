//! Departure and arrival boards: an airport's flights around now, with
//! their live status.

use std::cmp::Ordering;

use super::Operations;
use crate::{
    Id, SimDuration, SimTime,
    ops::{Callsign, Phase, timeline::place},
    schedule::Tail,
    time::LocalDate,
    world::Airport,
};

/// Nominal taxi-in added to a booked landing for the expected on-block time.
const TAXI_IN: SimDuration = SimDuration::from_minutes(5);
/// Boards keep departed and landed flights this long.
const KEEP: SimDuration = SimDuration::from_minutes(15);
/// Boards look this far ahead.
const HORIZON: SimDuration = SimDuration::from_hours(12);

/// Which side of an airport's traffic a board shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoardKind {
    /// Flights leaving the airport.
    Departures,
    /// Flights arriving at the airport.
    Arrivals,
}

/// Where a flight on a board stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightStatus {
    /// Not yet boarding.
    Scheduled,
    /// Boarding at the gate.
    Boarding,
    /// Left the gate, on the way.
    Departed,
    /// Touched down at the destination.
    Landed,
}

/// One line of a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardEntry {
    callsign: Callsign,
    tail: Id<Tail>,
    other: Id<Airport>,
    scheduled: SimTime,
    estimated: SimTime,
    status: FlightStatus,
    regulated: bool,
}

impl BoardEntry {
    /// Airline code and flight number.
    pub const fn callsign(&self) -> Callsign {
        self.callsign
    }

    /// The aircraft flying it.
    pub const fn tail(&self) -> Id<Tail> {
        self.tail
    }

    /// Destination on a departure board, origin on an arrival board.
    pub const fn other(&self) -> Id<Airport> {
        self.other
    }

    /// Scheduled off-block (departures) or on-block (arrivals) time.
    pub const fn scheduled(&self) -> SimTime {
        self.scheduled
    }

    /// Expected time; equal to the scheduled time until a delay is known.
    pub const fn estimated(&self) -> SimTime {
        self.estimated
    }

    /// Delay of the estimate over the schedule.
    pub const fn delay(&self) -> SimDuration {
        self.estimated.duration_since(self.scheduled)
    }

    /// Where the flight stands.
    pub const fn status(&self) -> FlightStatus {
        self.status
    }

    /// Held at the gate for a calculated take-off time because its
    /// destination is congested (D58).
    pub const fn regulated(&self) -> bool {
        self.regulated
    }
}

impl Operations {
    /// Up to `count` of `airport`'s departures or arrivals by scheduled time:
    /// those not yet gone, and finished ones from the last quarter of an hour.
    pub(crate) fn board(
        &self,
        airport: Id<Airport>,
        kind: BoardKind,
        count: usize,
    ) -> Vec<BoardEntry> {
        let from = self.time.checked_sub(KEEP).unwrap_or(self.time);
        let until = self.time.checked_add(HORIZON).unwrap_or(self.time);
        let mut entries = Vec::with_capacity(count * 2);
        for (index, tail) in self.tails.iter().enumerate() {
            let Ok(tail_id) = Id::<Tail>::from_usize(index) else {
                continue;
            };
            let flights = self.schedule.flights_of(tail_id);
            for (leg, flight) in flights.iter().enumerate() {
                let here = match kind {
                    BoardKind::Departures => flight.origin() == airport,
                    BoardKind::Arrivals => flight.destination() == airport,
                };
                if !here {
                    continue;
                }
                for weeks in [-7, 0, 7] {
                    let Ok(week) = tail.week.add_days(weeks) else {
                        continue;
                    };
                    let Ok(departure) = place(&self.zones[index], week, flight.departure()) else {
                        continue;
                    };
                    let Ok(arrival) = departure.checked_add(flight.block()) else {
                        continue;
                    };
                    let scheduled = match kind {
                        BoardKind::Departures => departure,
                        BoardKind::Arrivals => arrival,
                    };
                    if scheduled > until {
                        continue;
                    }
                    let (status, estimated, regulated) =
                        self.status(index, (week, leg), kind, scheduled);
                    // Finished flights leave the board after a while; late
                    // ones stay until they go.
                    let done = match kind {
                        BoardKind::Departures => {
                            matches!(status, FlightStatus::Departed | FlightStatus::Landed)
                        }
                        BoardKind::Arrivals => status == FlightStatus::Landed,
                    };
                    if scheduled < from && done {
                        continue;
                    }
                    let Some(airline) = self.callsigns.get(flight.airline().as_usize()) else {
                        continue;
                    };
                    entries.push(BoardEntry {
                        callsign: Callsign::new(*airline, flight.number()),
                        tail: tail_id,
                        other: match kind {
                            BoardKind::Departures => flight.destination(),
                            BoardKind::Arrivals => flight.origin(),
                        },
                        scheduled,
                        estimated,
                        status,
                        regulated,
                    });
                }
            }
        }
        entries.sort_by_key(|entry| {
            (
                entry.scheduled,
                entry.callsign.airline(),
                entry.callsign.number(),
            )
        });
        entries.truncate(count);
        entries
    }

    /// Status, estimated time and slot regulation of leg instance
    /// `(week, leg)` of tail `index`, scheduled at `scheduled` on this board.
    fn status(
        &self,
        index: usize,
        instance: (LocalDate, usize),
        kind: BoardKind,
        scheduled: SimTime,
    ) -> (FlightStatus, SimTime, bool) {
        let tail = &self.tails[index];
        match instance.cmp(&(tail.week, tail.leg)) {
            // A leg before the tail's current one is over.
            Ordering::Less => match kind {
                BoardKind::Departures => (FlightStatus::Departed, scheduled, false),
                BoardKind::Arrivals => (FlightStatus::Landed, scheduled, false),
            },
            Ordering::Greater => (FlightStatus::Scheduled, scheduled, false),
            Ordering::Equal => {
                let delay = tail.off_block.duration_since(tail.scheduled);
                let status = match tail.motion.map(|motion| motion.phase()) {
                    None => FlightStatus::Scheduled,
                    Some(Phase::Boarding) => FlightStatus::Boarding,
                    Some(Phase::Landing | Phase::TaxiIn | Phase::Arrived) => FlightStatus::Landed,
                    Some(_) => FlightStatus::Departed,
                };
                let flow = &self.flows[index];
                // A sequenced arrival is expected at its landing slot plus
                // taxi.
                let expected = match (kind, flow.landing) {
                    (BoardKind::Arrivals, Some(landing)) => landing.checked_add(TAXI_IN).ok(),
                    _ => scheduled.checked_add(delay).ok(),
                };
                let regulated = kind == BoardKind::Departures
                    && flow.slot_time.is_some()
                    && matches!(status, FlightStatus::Scheduled | FlightStatus::Boarding);
                (
                    status,
                    expected.unwrap_or(scheduled).max(scheduled),
                    regulated,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use crate::{
        RngSeed, Scenario, SimConfig, SimDuration, Simulation,
        ops::{BoardKind, FlightStatus},
        test_data::data,
    };

    fn germany() -> Result<Simulation> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?;
        Simulation::new(
            SimConfig::new("2026-07-08T10:00:00Z".parse()?, RngSeed::new(4)),
            scenario,
        )
    }

    #[test]
    fn test_departure_board_lists_upcoming_departures_in_order() -> Result<()> {
        let sim = germany()?;
        let frankfurt = sim
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let board = sim.board(frankfurt, BoardKind::Departures, 12);
        assert_eq!(board.len(), 12);
        let earliest = sim.time().checked_sub(SimDuration::from_minutes(15))?;
        for entry in &board {
            assert!(entry.scheduled() >= earliest);
            assert!(entry.estimated() >= entry.scheduled());
            assert_ne!(entry.other(), frankfurt);
            assert!(entry.callsign().to_string().len() >= 4);
        }
        assert!(
            board
                .windows(2)
                .all(|pair| pair[0].scheduled() <= pair[1].scheduled())
        );
        // Midday at FRA: something is boarding or has just left.
        assert!(
            board
                .iter()
                .any(|e| matches!(e.status(), FlightStatus::Boarding | FlightStatus::Departed))
        );
        Ok(())
    }

    #[test]
    fn test_arrival_board_progresses_with_time() -> Result<()> {
        let mut sim = germany()?;
        let munich = sim
            .world()
            .airport_by_ident("EDDM".parse()?)
            .context("EDDM")?;
        let before = sim.board(munich, BoardKind::Arrivals, 30);
        assert_eq!(before.len(), 30);
        let landed_before = before
            .iter()
            .filter(|e| e.status() == FlightStatus::Landed)
            .count();
        sim.run_ticks(1_200)?;
        let after = sim.board(munich, BoardKind::Arrivals, 30);
        let landed_after = after
            .iter()
            .filter(|e| e.status() == FlightStatus::Landed)
            .count();
        assert!(landed_after > 0, "{landed_before} → {landed_after}");
        Ok(())
    }

    #[test]
    fn test_board_is_empty_without_traffic() -> Result<()> {
        let sim = Simulation::new(
            SimConfig::new("2026-07-08T10:00:00Z".parse()?, RngSeed::new(1)),
            Scenario::empty(),
        )?;
        assert_eq!(
            sim.board(crate::Id::new(0), BoardKind::Departures, 5).len(),
            0
        );
        Ok(())
    }

    #[test]
    fn test_boards_show_sequenced_arrivals_and_slot_departures() -> Result<()> {
        let data = data()?;
        let mut definition = data.traffic.to_definition();
        for target in &mut definition.airports {
            if target.airport.to_string() == "EDDF" {
                for mode in &mut target.runway_modes {
                    mode.arrivals.truncate(1);
                    mode.departures = mode.arrivals.clone();
                }
            }
        }
        let traffic = crate::traffic::TrafficTargets::from_definition(definition)?;
        let scenario = Scenario::with_traffic(data.world, data.aircraft, data.airlines, traffic)?;
        // Seed 1 congests the single runway enough for ground delays.
        let mut sim = Simulation::new(
            SimConfig::new("2026-07-08T10:00:00Z".parse()?, RngSeed::new(1)),
            scenario,
        )?;
        sim.run_ticks(3_600)?;
        let frankfurt = sim
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let arrivals = sim.board(frankfurt, BoardKind::Arrivals, 60);
        assert!(
            arrivals
                .iter()
                .any(|e| e.status() == FlightStatus::Departed && e.estimated() > e.scheduled()),
            "sequenced arrivals show their expected delay"
        );
        // Every flight held for a slot shows as regulated on its origin's
        // board.
        let mut regulated = 0;
        for _ in 0..6 {
            sim.run_ticks(600)?;
            for view in sim.flights().filter(|view| view.slot_time().is_some()) {
                let board = sim.board(view.origin(), BoardKind::Departures, 400);
                let entry = board
                    .iter()
                    .find(|entry| entry.tail() == view.tail())
                    .context("a held flight stays on the board")?;
                assert!(entry.regulated());
                regulated += 1;
            }
        }
        assert!(regulated > 0, "departures to FRA wait for slots");
        Ok(())
    }
}
