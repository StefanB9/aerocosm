//! Choosing the runway in use (D31).

use crate::{
    units::{Degrees, Meters},
    world::Runway,
};

/// The wind every airport operates into until weather arrives: the prevailing
/// south-westerly over Germany.
pub(crate) const PREVAILING_WIND: Degrees = Degrees::from_raw(250.0);

/// A runway (index into the airport's runways) and the end aircraft use: they
/// take off from and land at that end's threshold, heading along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunwayChoice {
    pub(crate) runway: usize,
    pub(crate) end: usize,
}

/// Smallest angle between two directions, `0..=180` degrees.
fn angle_between(a: Degrees, b: Degrees) -> f64 {
    let difference = (a.value() - b.value()).rem_euclid(360.0);
    difference.min(360.0 - difference)
}

/// The runway end best aligned into `wind_from` among runways at least
/// `min_length` long, or among all runways if none is; `None` without runways.
pub(crate) fn runway_in_use(
    runways: &[Runway],
    wind_from: Degrees,
    min_length: Meters,
) -> Option<RunwayChoice> {
    let long_enough = runways.iter().any(|runway| runway.length() >= min_length);
    let mut best: Option<(f64, RunwayChoice)> = None;
    for (index, runway) in runways.iter().enumerate() {
        if long_enough && runway.length() < min_length {
            continue;
        }
        for (end, threshold) in runway.ends().iter().enumerate() {
            let misalignment = angle_between(threshold.true_heading(), wind_from);
            if best.is_none_or(|(least, _)| misalignment < least) {
                best = Some((misalignment, RunwayChoice { runway: index, end }));
            }
        }
    }
    best.map(|(_, choice)| choice)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;
    use crate::{
        Id, LatLon,
        world::{GeometrySource, RunwayDefinition, RunwayEnd, Surface},
    };

    /// A runway centred near Frankfurt with its low end heading `heading`.
    fn runway(low: &str, heading: f64, length: f64) -> Result<Runway> {
        let designator: crate::world::RunwayDesignator = low.parse()?;
        let centre = LatLon::new(50.03, 8.57)?;
        let heading = Degrees::new(heading)?;
        let back = Degrees::new(heading.value() + 180.0)?.normalized();
        let half = Meters::new(length / 2.0)?;
        let definition = RunwayDefinition {
            airport: "EDDF".parse()?,
            ends: [
                RunwayEnd::new(
                    designator,
                    centre.destination(back, half),
                    heading,
                    Meters::default(),
                )?,
                RunwayEnd::new(
                    designator.reciprocal(),
                    centre.destination(heading, half),
                    back,
                    Meters::default(),
                )?,
            ],
            length: Meters::new(length)?,
            width: Meters::new(45.0)?,
            surface: Surface::Paved,
            geometry: GeometrySource::Surveyed,
        };
        Runway::new(Id::new(0), &definition)
    }

    #[test]
    fn test_runway_in_use_faces_the_wind() -> Result<()> {
        let runways = [runway("07", 70.0, 4000.0)?, runway("18", 180.0, 4000.0)?];
        let choice = runway_in_use(&runways, PREVAILING_WIND, Meters::new(2000.0)?);
        assert_eq!(choice, Some(RunwayChoice { runway: 0, end: 1 }));
        let north = runway_in_use(&runways, Degrees::new(10.0)?, Meters::new(2000.0)?);
        assert_eq!(north, Some(RunwayChoice { runway: 1, end: 1 }));
        Ok(())
    }

    #[test]
    fn test_runway_in_use_skips_short_runways() -> Result<()> {
        let runways = [runway("07", 70.0, 1200.0)?, runway("18", 180.0, 3000.0)?];
        let choice = runway_in_use(&runways, PREVAILING_WIND, Meters::new(2000.0)?);
        assert_eq!(choice, Some(RunwayChoice { runway: 1, end: 0 }));
        Ok(())
    }

    #[test]
    fn test_runway_in_use_falls_back_to_any_runway() -> Result<()> {
        let runways = [runway("07", 70.0, 1200.0)?];
        let choice = runway_in_use(&runways, PREVAILING_WIND, Meters::new(2000.0)?);
        assert_eq!(choice, Some(RunwayChoice { runway: 0, end: 1 }));
        assert_eq!(runway_in_use(&[], PREVAILING_WIND, Meters::new(1.0)?), None);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_runway_in_use_picks_the_least_misaligned_end(
            first in 0.0_f64..360.0, second in 0.0_f64..360.0, wind in 0.0_f64..360.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let runways = [
                runway("01", first, 3000.0).map_err(fail)?,
                runway("02", second, 3000.0).map_err(fail)?,
            ];
            let wind = Degrees::new(wind).map_err(fail)?;
            let choice = runway_in_use(&runways, wind, Meters::new(2500.0).map_err(fail)?)
                .ok_or_else(|| TestCaseError::fail("no choice"))?;
            let chosen = angle_between(runways[choice.runway].ends()[choice.end].true_heading(), wind);
            prop_assert!(chosen <= 90.0 + 1e-9);
            for runway in &runways {
                for end in runway.ends() {
                    prop_assert!(chosen <= angle_between(end.true_heading(), wind) + 1e-9);
                }
            }
        }
    }
}
