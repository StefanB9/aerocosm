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
}

impl Operations {
    /// Up to `count` of `airport`'s departures or arrivals from a quarter of
    /// an hour ago onwards, by scheduled time.
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
                    if scheduled < from || scheduled > until {
                        continue;
                    }
                    let (status, delay) = self.status(index, (week, leg), kind);
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
                        estimated: scheduled.checked_add(delay).unwrap_or(scheduled),
                        status,
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

    /// Status and known delay of leg instance `(week, leg)` of tail `index`.
    fn status(
        &self,
        index: usize,
        instance: (LocalDate, usize),
        kind: BoardKind,
    ) -> (FlightStatus, SimDuration) {
        let tail = &self.tails[index];
        match instance.cmp(&(tail.week, tail.leg)) {
            // A leg before the tail's current one is over.
            Ordering::Less => match kind {
                BoardKind::Departures => (FlightStatus::Departed, SimDuration::ZERO),
                BoardKind::Arrivals => (FlightStatus::Landed, SimDuration::ZERO),
            },
            Ordering::Greater => (FlightStatus::Scheduled, SimDuration::ZERO),
            Ordering::Equal => {
                let delay = tail.off_block.duration_since(tail.scheduled);
                let status = match tail.motion.map(|motion| motion.phase()) {
                    None => FlightStatus::Scheduled,
                    Some(Phase::Boarding) => FlightStatus::Boarding,
                    Some(Phase::Landing | Phase::TaxiIn | Phase::Arrived) => FlightStatus::Landed,
                    Some(_) => FlightStatus::Departed,
                };
                (status, delay)
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
}
