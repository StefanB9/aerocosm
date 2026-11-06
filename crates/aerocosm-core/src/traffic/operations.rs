//! Declared capacities and runway operating modes (plan M3, D54/D55).

use std::collections::HashSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{CoreError, units::Degrees, world::RunwayDesignator};

/// A coordinated airport's declared capacity: scheduled movements (arrivals
/// plus departures) per hour and per shorter window (D54).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    /// Movements per clock hour and per rolling hour.
    pub per_hour: u16,
    /// Length of the short window, a divisor of 60 below 60.
    pub window_minutes: u16,
    /// Movements per short window.
    pub per_window: u16,
}

impl Capacity {
    /// Checks the limits are positive and consistent.
    pub fn validate(&self) -> Result<()> {
        let invalid = |reason| CoreError::InvalidTraffic { reason };
        if self.per_hour == 0 || self.per_window == 0 {
            return Err(invalid("capacities must be positive").into());
        }
        if self.per_window > self.per_hour {
            return Err(invalid("a window cannot hold more than its hour").into());
        }
        if self.window_minutes == 0 || self.window_minutes >= 60 || 60 % self.window_minutes != 0 {
            return Err(invalid("capacity windows must divide the hour").into());
        }
        Ok(())
    }

    /// Short windows per hour.
    pub const fn windows_per_hour(&self) -> u16 {
        60 / self.window_minutes
    }
}

/// Runways used for arrivals and departures when operating towards `flow`,
/// the direction the wind blows from (D55).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunwayMode {
    /// Wind direction this mode operates into, `[0, 360)`.
    pub flow: Degrees,
    /// Runway ends used for landing.
    pub arrivals: Vec<RunwayDesignator>,
    /// Runway ends used for take-off.
    pub departures: Vec<RunwayDesignator>,
}

impl RunwayMode {
    /// Checks the flow and that both runway lists are non-empty and distinct.
    pub fn validate(&self) -> Result<()> {
        let invalid = |reason| CoreError::InvalidTraffic { reason };
        if !(0.0..360.0).contains(&self.flow.value()) {
            return Err(invalid("runway mode flow must be within [0, 360)").into());
        }
        for list in [&self.arrivals, &self.departures] {
            let unique: HashSet<_> = list.iter().collect();
            if list.is_empty() || unique.len() != list.len() {
                return Err(invalid("runway mode lists must be non-empty and distinct").into());
            }
        }
        Ok(())
    }
}

/// Smallest angle between two directions, `0..=180` degrees.
pub(crate) fn angle_between(a: Degrees, b: Degrees) -> f64 {
    let difference = (a.value() - b.value()).rem_euclid(360.0);
    difference.min(360.0 - difference)
}

/// The mode whose flow is closest to `wind`, the first among equals.
pub fn mode_for_wind(modes: &[RunwayMode], wind: Degrees) -> Option<&RunwayMode> {
    modes
        .iter()
        .fold(None, |best: Option<&RunwayMode>, mode| match best {
            Some(best) if angle_between(best.flow, wind) <= angle_between(mode.flow, wind) => {
                Some(best)
            }
            _ => Some(mode),
        })
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use proptest::prelude::*;

    use super::*;

    fn designators(list: &[&str]) -> Result<Vec<RunwayDesignator>> {
        list.iter().map(|text| text.parse()).collect()
    }

    #[test]
    fn test_capacity_validation() -> Result<()> {
        let fra = Capacity {
            per_hour: 104,
            window_minutes: 15,
            per_window: 30,
        };
        fra.validate()?;
        for bad in [
            Capacity { per_hour: 0, ..fra },
            Capacity {
                per_window: 0,
                ..fra
            },
            Capacity {
                per_window: 105,
                ..fra
            },
            Capacity {
                window_minutes: 0,
                ..fra
            },
            Capacity {
                window_minutes: 7,
                ..fra
            },
            Capacity {
                window_minutes: 60,
                ..fra
            },
        ] {
            if bad.validate().is_ok() {
                bail!("{bad:?} must be rejected");
            }
        }
        Ok(())
    }

    #[test]
    fn test_capacity_serde_round_trip() -> Result<()> {
        let json = r#"{"per_hour":90,"window_minutes":10,"per_window":15}"#;
        let capacity: Capacity = serde_json::from_str(json)?;
        assert_eq!(capacity.windows_per_hour(), 6);
        assert_eq!(serde_json::to_string(&capacity)?, json);
        Ok(())
    }

    #[test]
    fn test_runway_mode_validation() -> Result<()> {
        let west = RunwayMode {
            flow: Degrees::new(250.0)?,
            arrivals: designators(&["25R", "25L"])?,
            departures: designators(&["25C", "18"])?,
        };
        west.validate()?;
        let empty = RunwayMode {
            arrivals: Vec::new(),
            ..west.clone()
        };
        assert!(empty.validate().is_err());
        let duplicate = RunwayMode {
            departures: designators(&["25C", "25C"])?,
            ..west.clone()
        };
        assert!(duplicate.validate().is_err());
        let out_of_range = RunwayMode {
            flow: Degrees::new(360.0)?,
            ..west
        };
        assert!(out_of_range.validate().is_err());
        Ok(())
    }

    #[test]
    fn test_mode_for_wind_picks_the_closest_flow() -> Result<()> {
        let west = RunwayMode {
            flow: Degrees::new(250.0)?,
            arrivals: designators(&["25L"])?,
            departures: designators(&["25L"])?,
        };
        let east = RunwayMode {
            flow: Degrees::new(70.0)?,
            arrivals: designators(&["07R"])?,
            departures: designators(&["07R"])?,
        };
        let modes = [west.clone(), east.clone()];
        assert_eq!(mode_for_wind(&modes, Degrees::new(250.0)?), Some(&west));
        assert_eq!(mode_for_wind(&modes, Degrees::new(120.0)?), Some(&east));
        assert_eq!(mode_for_wind(&[], Degrees::new(120.0)?), None);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_mode_for_wind_minimises_the_angle(
            flows in prop::collection::vec(0.0_f64..360.0, 1..5),
            wind in 0.0_f64..360.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let modes: Vec<RunwayMode> = flows
                .iter()
                .map(|flow| Ok(RunwayMode {
                    flow: Degrees::new(*flow)?,
                    arrivals: designators(&["01"])?,
                    departures: designators(&["01"])?,
                }))
                .collect::<Result<_>>()
                .map_err(fail)?;
            let wind = Degrees::new(wind).map_err(fail)?;
            let chosen = mode_for_wind(&modes, wind)
                .ok_or_else(|| TestCaseError::fail("no mode"))?;
            for mode in &modes {
                prop_assert!(angle_between(chosen.flow, wind) <= angle_between(mode.flow, wind) + 1e-9);
            }
        }
    }
}
