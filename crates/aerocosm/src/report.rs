//! Checking a run's movements against the calibration targets.

use aerocosm_core::{
    SimDuration, SimTime, Simulation,
    ops::AirportStats,
    time::Weekday,
    traffic::{AirportTarget, Capacity, TrafficTargets},
    world::codes::AirportIdent,
};

/// Actual hourly movements may exceed the declared capacity by this fraction
/// before the run warns.
const CAPACITY_TOLERANCE: f64 = 0.05;

/// Movements are expected hour by hour, by the local weekday of each hour.
const STEP: SimDuration = SimDuration::from_hours(1);

/// Movements the targets call for at `target`'s airport between `start` and
/// `start + duration`.
pub(crate) fn expected_movements(
    target: &AirportTarget,
    factors: &[f64; 7],
    start: SimTime,
    duration: SimDuration,
) -> f64 {
    let mut expected = 0.0;
    let mut time = start;
    let end = start.checked_add(duration).unwrap_or(start);
    while time < end {
        let step = end.duration_since(time).min(STEP);
        let weekday: Weekday = target.zone().local(time).weekday();
        expected += target.movements_per_day(weekday, factors) * step.as_secs_f64() / 86_400.0;
        let Ok(next) = time.checked_add(step) else {
            break;
        };
        time = next;
    }
    expected
}

/// One airport's movements in a run against its target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AirportReport {
    pub(crate) airport: AirportIdent,
    pub(crate) actual: u32,
    pub(crate) expected: f64,
    /// Punctuality, delays and hourly movements at the airport.
    pub(crate) stats: AirportStats,
    /// Declared capacity, if the airport is coordinated.
    pub(crate) capacity: Option<Capacity>,
}

impl AirportReport {
    /// Relative deviation from the target; zero without a target.
    pub(crate) fn deviation(&self) -> f64 {
        if self.expected <= 0.0 {
            return 0.0;
        }
        f64::from(self.actual) / self.expected - 1.0
    }

    /// The busiest clock hour and the declared hourly capacity, if the
    /// airport is coordinated and that hour exceeded it by more than
    /// [`CAPACITY_TOLERANCE`]: schedules stay within capacity, but delays
    /// bunch actual movements a little, as at real airports.
    pub(crate) fn capacity_exceeded(&self) -> Option<(u32, u16)> {
        let capacity = self.capacity?;
        let busiest = self.stats.busiest_hour();
        let limit = f64::from(capacity.per_hour) * (1.0 + CAPACITY_TOLERANCE);
        (f64::from(busiest) > limit).then_some((busiest, capacity.per_hour))
    }

    /// Whether the movements are within `tolerance` of the target, allowing
    /// two movements per simulated day for rounding at small airports.
    pub(crate) fn within(&self, tolerance: f64, days: f64) -> bool {
        (f64::from(self.actual) - self.expected).abs() <= self.expected * tolerance + 2.0 * days
    }
}

/// Every target airport's movements in `simulation` since `start`.
pub(crate) fn airport_reports(
    simulation: &Simulation,
    targets: &TrafficTargets,
    start: SimTime,
) -> Vec<AirportReport> {
    let duration = simulation.time().duration_since(start);
    targets
        .airports()
        .map(|target| AirportReport {
            airport: target.airport(),
            stats: simulation
                .world()
                .airport_by_ident(target.airport())
                .zip(simulation.statistics())
                .map_or_else(AirportStats::default, |(id, stats)| stats.airport(id)),
            capacity: target.capacity(),
            actual: simulation
                .world()
                .airport_by_ident(target.airport())
                .map_or(0, |id| simulation.movements(id)),
            expected: expected_movements(target, targets.weekday_factors(), start, duration),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{
        time::Zone,
        traffic::{AirportTarget, TrafficDefinition, TrafficTargets},
    };
    use anyhow::{Context, Result};

    use super::*;

    fn targets() -> Result<TrafficTargets> {
        TrafficTargets::from_definition(TrafficDefinition {
            source: "test fixture".to_owned(),
            weekday_factors: [1.03, 0.97, 0.98, 1.02, 1.06, 0.88, 1.06],
            airports: vec![AirportTarget {
                airport: "EDDF".parse()?,
                movements_per_year: 502_634,
                zone: Zone::new("Europe/Berlin")?,
                curfew: None,
                banks: Vec::new(),
                cargo_share: None,
                capacity: None,
                runway_modes: Vec::new(),
            }],
        })
    }

    #[test]
    fn test_expected_movements_over_a_week_match_the_yearly_rate() -> Result<()> {
        let targets = targets()?;
        let frankfurt = targets.airport("EDDF".parse()?).context("EDDF")?;
        let week = expected_movements(
            frankfurt,
            targets.weekday_factors(),
            "2026-07-06T00:00:00Z".parse()?,
            SimDuration::from_hours(7 * 24),
        );
        let wanted = f64::from(frankfurt.movements_per_year()) / 365.0 * 7.0;
        assert!((week - wanted).abs() < wanted * 1e-6, "{week} vs {wanted}");
        Ok(())
    }

    #[test]
    fn test_expected_movements_follow_the_local_weekday() -> Result<()> {
        let targets = targets()?;
        let frankfurt = targets.airport("EDDF".parse()?).context("EDDF")?;
        let factors = targets.weekday_factors();
        // 12:00-13:00 UTC on Monday 6 July is Monday afternoon in Berlin.
        let hour = expected_movements(
            frankfurt,
            factors,
            "2026-07-06T12:00:00Z".parse()?,
            SimDuration::from_hours(1),
        );
        let monday = frankfurt.movements_per_day(Weekday::Monday, factors) / 24.0;
        assert!((hour - monday).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_airport_report_tolerance() -> Result<()> {
        let report = AirportReport {
            airport: "EDDF".parse()?,
            actual: 1_030,
            expected: 1_000.0,
            stats: AirportStats::default(),
            capacity: None,
        };
        assert!((report.deviation() - 0.03).abs() < 1e-12);
        assert!(report.within(0.05, 1.0));
        let small = AirportReport {
            airport: "EDDR".parse()?,
            actual: 6,
            expected: 9.0,
            stats: AirportStats::default(),
            capacity: None,
        };
        // 3 off is 33 %, but within the allowance of 2 movements per day for
        // a two-day run.
        assert!(small.within(0.05, 2.0));
        assert!(!small.within(0.05, 1.0));
        Ok(())
    }

    #[test]
    fn test_capacity_check_compares_the_busiest_hour() -> Result<()> {
        let targets = targets()?;
        let frankfurt = targets.airport("EDDF".parse()?).context("EDDF")?;
        let report = AirportReport {
            airport: frankfurt.airport(),
            actual: 0,
            expected: 0.0,
            stats: AirportStats::default(),
            capacity: frankfurt.capacity(),
        };
        assert_eq!(report.capacity_exceeded(), None);
        let uncoordinated = AirportReport {
            capacity: None,
            ..report
        };
        assert_eq!(uncoordinated.capacity_exceeded(), None);
        Ok(())
    }
}
