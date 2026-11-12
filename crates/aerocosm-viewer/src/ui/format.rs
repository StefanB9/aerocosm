//! Text for panels and boards.

use aerocosm_core::{
    SimDuration, SimTime,
    ops::{FlightStatus, Phase, TaxiGoal, TaxiState},
    time::Zone,
    units::Feet,
};

/// Below this altitude levels read in feet, above as flight levels.
const TRANSITION_FEET: f64 = 6_000.0;

/// `hh:mm` of a minute of the day.
pub(crate) fn clock(minute_of_day: u16) -> String {
    format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// Local `hh:mm` at an airport in `zone`, or UTC with a marker without one.
pub(crate) fn local_clock(zone: Option<&Zone>, time: SimTime) -> String {
    zone.map_or_else(
        || {
            let civil = time.to_civil();
            format!("{:02}:{:02}Z", civil.hour(), civil.minute())
        },
        |zone| clock(zone.local(time).minute_of_day()),
    )
}

/// `FL360` aloft, `3,200 ft` low down, `ground` on the ground.
pub(crate) fn altitude(feet: Feet, phase: Phase) -> String {
    if !phase.is_airborne() {
        return "ground".to_owned();
    }
    let feet = feet.value().max(0.0);
    if feet >= TRANSITION_FEET {
        // Flight levels are hundreds of feet, rounded.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let level = (feet / 100.0).round() as u32;
        format!("FL{level:03}")
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rounded = ((feet / 100.0).round() * 100.0) as u32;
        let (thousands, rest) = (rounded / 1000, rounded % 1000);
        if thousands > 0 {
            format!("{thousands},{rest:03} ft")
        } else {
            format!("{rest} ft")
        }
    }
}

/// A phase as shown in the flight panel.
pub(crate) const fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Boarding => "Boarding",
        Phase::Pushback => "Pushing back",
        Phase::TaxiOut => "Taxiing out",
        Phase::Queue => "Holding short",
        Phase::LineUp => "Lining up",
        Phase::TakeoffRoll => "Taking off",
        Phase::Climb => "Climbing",
        Phase::Cruise => "Cruising",
        Phase::Holding => "Holding",
        Phase::Descent => "Descending",
        Phase::Approach => "On approach",
        Phase::Landing => "Landing",
        Phase::TaxiIn => "Taxiing in",
        Phase::Arrived => "Arrived",
    }
}

/// Where a taxi goes, as in "to holding point 25C".
fn taxi_goal(goal: TaxiGoal) -> String {
    match goal {
        TaxiGoal::HoldingPoint(Some(runway)) => format!("holding point {runway}"),
        TaxiGoal::HoldingPoint(None) => "the holding point".to_owned(),
        TaxiGoal::Stand(stand) => format!("stand {stand}"),
        TaxiGoal::Spot => "a waiting spot".to_owned(),
    }
}

/// What ground control does with an aircraft, for the flight panel (plan
/// M4 D85).
pub(crate) fn taxi_state(state: TaxiState) -> String {
    match state {
        TaxiState::Pushback { stand: Some(stand) } => format!("Pushback from {stand}"),
        TaxiState::Pushback { stand: None } => "Pushback".to_owned(),
        TaxiState::Taxiing { via, to } => {
            let names: Vec<String> = via.iter().flatten().map(ToString::to_string).collect();
            if names.is_empty() {
                format!("Taxiing to {}", taxi_goal(to))
            } else {
                format!("Taxiing via {} to {}", names.join(", "), taxi_goal(to))
            }
        }
        TaxiState::HoldingShort {
            runway: Some(runway),
        } => format!("Holding short of runway {runway}"),
        TaxiState::HoldingShort { runway: None } => "Holding short of a runway".to_owned(),
        TaxiState::WaitingFor { flight, to } => {
            format!("Waiting for {flight}, then to {}", taxi_goal(to))
        }
        TaxiState::AtHoldingPoint {
            runway: Some(runway),
        } => format!("At holding point {runway}"),
        TaxiState::AtHoldingPoint { runway: None } => "At the holding point".to_owned(),
        TaxiState::WaitingForStand => "Waiting at a spot for a stand".to_owned(),
        TaxiState::ClearedRegardless { to } => format!(
            "Taxiing regardless of traffic to {} (ground conflict)",
            taxi_goal(to)
        ),
    }
}

/// `+7 min` for delays of a minute or more.
pub(crate) fn delay(delay: SimDuration) -> Option<String> {
    let minutes = delay.as_millis() / 60_000;
    (minutes >= 1).then(|| format!("+{minutes} min"))
}

/// A board's status column.
pub(crate) fn status(status: FlightStatus, late: SimDuration) -> String {
    let base = match status {
        FlightStatus::Scheduled => "Scheduled",
        FlightStatus::Boarding => "Boarding",
        FlightStatus::Departed => "Departed",
        FlightStatus::Landed => "Landed",
    };
    match (status, delay(late)) {
        (FlightStatus::Scheduled | FlightStatus::Boarding, Some(late)) => format!("Delayed {late}"),
        _ => base.to_owned(),
    }
}

/// A board's status for a flight held for a slot at `time`.
pub(crate) fn slot(time: &str) -> String {
    format!("Slot {time}")
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{ops::Callsign, schedule::FlightNumber};
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_taxi_state_reads_like_a_ground_clearance() -> Result<()> {
        let (n, n7) = ("N".parse()?, "N7".parse()?);
        let hold = TaxiGoal::HoldingPoint(Some("25C".parse()?));
        let stand = TaxiGoal::Stand("A14".parse()?);
        assert_eq!(
            taxi_state(TaxiState::Pushback {
                stand: Some("V155".parse()?)
            }),
            "Pushback from V155"
        );
        assert_eq!(taxi_state(TaxiState::Pushback { stand: None }), "Pushback");
        assert_eq!(
            taxi_state(TaxiState::Taxiing {
                via: [Some(n), Some(n7), None],
                to: hold,
            }),
            "Taxiing via N, N7 to holding point 25C"
        );
        assert_eq!(
            taxi_state(TaxiState::Taxiing {
                via: [None; 3],
                to: stand,
            }),
            "Taxiing to stand A14"
        );
        assert_eq!(
            taxi_state(TaxiState::HoldingShort {
                runway: Some("07C/25C".parse()?)
            }),
            "Holding short of runway 07C/25C"
        );
        assert_eq!(
            taxi_state(TaxiState::WaitingFor {
                flight: Callsign::new("DLH".parse()?, FlightNumber::new(123)),
                to: TaxiGoal::Spot,
            }),
            "Waiting for DLH123, then to a waiting spot"
        );
        assert_eq!(
            taxi_state(TaxiState::AtHoldingPoint {
                runway: Some("25C".parse()?)
            }),
            "At holding point 25C"
        );
        assert_eq!(
            taxi_state(TaxiState::AtHoldingPoint { runway: None }),
            "At the holding point"
        );
        assert_eq!(
            taxi_state(TaxiState::WaitingForStand),
            "Waiting at a spot for a stand"
        );
        assert_eq!(
            taxi_state(TaxiState::ClearedRegardless { to: stand }),
            "Taxiing regardless of traffic to stand A14 (ground conflict)"
        );
        Ok(())
    }

    #[test]
    fn test_clock_and_local_clock() -> Result<()> {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(8 * 60 + 5), "08:05");
        let time: SimTime = "2026-11-05T07:05:00Z".parse()?;
        assert_eq!(
            local_clock(Some(&Zone::new("Europe/Berlin")?), time),
            "08:05"
        );
        assert_eq!(local_clock(None, time), "07:05Z");
        Ok(())
    }

    #[test]
    fn test_altitude_text() -> Result<()> {
        assert_eq!(altitude(Feet::new(36_020.0)?, Phase::Cruise), "FL360");
        assert_eq!(altitude(Feet::new(7_950.0)?, Phase::Climb), "FL080");
        assert_eq!(altitude(Feet::new(3_180.0)?, Phase::Approach), "3,200 ft");
        assert_eq!(altitude(Feet::new(450.0)?, Phase::Climb), "500 ft");
        assert_eq!(altitude(Feet::new(300.0)?, Phase::TaxiOut), "ground");
        Ok(())
    }

    #[test]
    fn test_phase_names_are_distinct() {
        use std::collections::HashSet;
        let phases = [
            Phase::Boarding,
            Phase::Pushback,
            Phase::TaxiOut,
            Phase::Queue,
            Phase::LineUp,
            Phase::TakeoffRoll,
            Phase::Climb,
            Phase::Cruise,
            Phase::Holding,
            Phase::Descent,
            Phase::Approach,
            Phase::Landing,
            Phase::TaxiIn,
            Phase::Arrived,
        ];
        let names: HashSet<_> = phases.iter().map(|phase| phase_name(*phase)).collect();
        assert_eq!(names.len(), phases.len());
    }

    #[test]
    fn test_delay_and_status_text() {
        assert_eq!(delay(SimDuration::from_secs(59)), None);
        assert_eq!(
            delay(SimDuration::from_minutes(12)),
            Some("+12 min".to_owned())
        );
        let late = SimDuration::from_minutes(12);
        assert_eq!(
            status(FlightStatus::Scheduled, SimDuration::ZERO),
            "Scheduled"
        );
        assert_eq!(status(FlightStatus::Scheduled, late), "Delayed +12 min");
        assert_eq!(status(FlightStatus::Boarding, late), "Delayed +12 min");
        assert_eq!(status(FlightStatus::Departed, late), "Departed");
        assert_eq!(status(FlightStatus::Landed, SimDuration::ZERO), "Landed");
        assert_eq!(slot("09:15"), "Slot 09:15");
    }
}
