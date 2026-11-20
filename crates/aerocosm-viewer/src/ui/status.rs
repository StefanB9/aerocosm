//! The airport panel's flow lines: runways in use, live queues and the day's
//! punctuality (plan M3 PR 5).

use std::fmt::Write as _;

use aerocosm_core::{Id, Simulation, ops::Phase, world::Airport};

/// Runways in use, aircraft holding for or queuing at `airport` now, its
/// punctuality since the start, its taxiing under ground control (plan
/// M4 D88) and its airlines' recovery (M8 D173); runway line only without
/// traffic.
pub(crate) fn status_lines(simulation: &Simulation, airport: Id<Airport>) -> Vec<String> {
    let mut lines = Vec::with_capacity(4);
    let runways = simulation.world().runways_of(airport);
    let mut landing = Vec::new();
    let mut departing = Vec::new();
    for active in simulation.runways_in_use(airport) {
        let Some(end) = runways
            .get(active.runway)
            .and_then(|r| r.ends().get(active.end))
        else {
            continue;
        };
        let name = end.designator().to_string();
        if active.arrivals {
            landing.push(name.clone());
        }
        if active.departures {
            departing.push(name);
        }
    }
    if !landing.is_empty() || !departing.is_empty() {
        lines.push(format!(
            "Landing {} · take-off {}",
            landing.join(" "),
            departing.join(" ")
        ));
    }
    let Some(stats) = simulation.statistics() else {
        return lines;
    };
    let (mut holding, mut queued, mut slots) = (0, 0, 0);
    for flight in simulation.flights() {
        if flight.destination() == airport && flight.phase() == Phase::Holding {
            holding += 1;
        }
        if flight.origin() == airport && flight.phase() == Phase::Queue {
            queued += 1;
        }
        if flight.origin() == airport && flight.slot_time().is_some() {
            slots += 1;
        }
    }
    lines.push(format!(
        "Now: {holding} holding · {queued} waiting for take-off · {slots} held for slots"
    ));
    let at = stats.airport(airport);
    let mut summary = String::with_capacity(80);
    let _ = write!(
        summary,
        "Since start: {:.0} % on time · avg delay {:.0} min · holding {:.0} min",
        at.on_time_departures() * 100.0,
        at.average_departure_delay().as_secs_f64() / 60.0,
        at.holding_minutes()
    );
    lines.push(summary);
    if at.taxis() > 0 {
        lines.push(format!(
            "Ground: {} taxis · taxi delay {:.0} min · waiting for clearance {:.0} min",
            at.taxis(),
            at.average_taxi_delay().as_secs_f64() / 60.0,
            at.average_clearance_wait().as_secs_f64() / 60.0
        ));
    }
    lines.extend(recovery_line(at.cancellations(), at.swaps(), at.spares()));
    lines
}

/// An airport's recovery since the start (plan M8 D173): cancelled
/// departures, swaps and spares; `None` before any.
pub(crate) fn recovery_line(cancelled: u32, swaps: u32, spares: u32) -> Option<String> {
    let plural = |count: u32, one: &str| {
        if count == 1 {
            format!("{count} {one}")
        } else {
            format!("{count} {one}s")
        }
    };
    (cancelled + swaps + spares > 0).then(|| {
        format!(
            "Recovery: {cancelled} cancelled · {} · {}",
            plural(swaps, "swap"),
            plural(spares, "spare")
        )
    })
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{RngSeed, SimConfig, Simulation};
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_status_lines_describe_frankfurt() -> Result<()> {
        let (scenario, _) = crate::data::load_with_ground(&['E'])?;
        let mut simulation = Simulation::new(
            SimConfig::new("2026-11-05T07:00:00Z".parse()?, RngSeed::new(0)),
            scenario,
        )?;
        simulation.run_ticks(1_800)?;
        let frankfurt = simulation
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let lines = status_lines(&simulation, frankfurt);
        assert_eq!(lines.len(), 4, "{lines:?}");
        // West flow by rule (plan M5 D113): two runways each way.
        let (landing, take_off) = lines[0].split_once(" · take-off ").context("both")?;
        let names = |text: &str| text.split(' ').filter(|n| n.starts_with("25")).count();
        assert!(landing.starts_with("Landing 25"), "{}", lines[0]);
        assert_eq!((names(landing), names(take_off)), (2, 2), "{}", lines[0]);
        assert!(lines[1].starts_with("Now: "), "{}", lines[1]);
        assert!(lines[2].contains("% on time"), "{}", lines[2]);
        assert!(lines[3].starts_with("Ground: "), "{}", lines[3]);
        assert!(lines[3].contains("waiting for clearance"), "{}", lines[3]);
        Ok(())
    }

    #[test]
    fn test_recovery_line_counts_cancellations_swaps_and_spares() {
        assert_eq!(recovery_line(0, 0, 0), None);
        assert_eq!(
            recovery_line(3, 1, 0).as_deref(),
            Some("Recovery: 3 cancelled · 1 swap · 0 spares")
        );
        assert_eq!(
            recovery_line(1, 2, 1).as_deref(),
            Some("Recovery: 1 cancelled · 2 swaps · 1 spare")
        );
    }

    #[test]
    fn test_status_lines_without_traffic_are_empty() -> Result<()> {
        let (scenario, _) = crate::data::load()?;
        let world = scenario.world().clone();
        let simulation = Simulation::new(
            SimConfig::new("2026-11-05T07:00:00Z".parse()?, RngSeed::new(0)),
            aerocosm_core::Scenario::new(world),
        )?;
        let lines = status_lines(&simulation, aerocosm_core::Id::new(0));
        assert!(lines.iter().all(|line| !line.contains("on time")));
        Ok(())
    }
}
