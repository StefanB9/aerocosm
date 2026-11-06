//! Which edge-to-edge routes cross the simulated airspace (plan D29).

use crate::LatLon;

/// Points sampled along a great circle; at most ~40 km apart for the longest
/// routes in the map, finer than the coarse airspace outline.
const SAMPLES: u32 = 64;

/// Whether `point` lies inside the closed `ring` (ray casting in
/// longitude/latitude, which is accurate enough for a coarse outline the size
/// of a country).
pub(crate) fn contains(ring: &[LatLon], point: LatLon) -> bool {
    let (x, y) = (point.longitude().value(), point.latitude().value());
    let mut inside = false;
    for edge in ring.windows(2) {
        let (x1, y1) = (edge[0].longitude().value(), edge[0].latitude().value());
        let (x2, y2) = (edge[1].longitude().value(), edge[1].latitude().value());
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) / (y2 - y1) * (x2 - x1) {
            inside = !inside;
        }
    }
    inside
}

/// Whether the great circle from `a` to `b` passes through the area inside
/// `ring`.
pub(crate) fn crosses(ring: &[LatLon], a: LatLon, b: LatLon) -> bool {
    (0..=SAMPLES).any(|step| {
        a.interpolate(b, f64::from(step) / f64::from(SAMPLES))
            .is_ok_and(|point| contains(ring, point))
    })
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{LatLon, traffic::TrafficTargets};

    fn germany() -> Result<Vec<LatLon>> {
        let targets =
            TrafficTargets::from_json(include_str!("../../../../assets/traffic/germany.json"))
                .context("traffic targets")?;
        Ok(targets.airspace().to_vec())
    }

    fn at(lat: f64, lon: f64) -> Result<LatLon> {
        LatLon::new(lat, lon)
    }

    #[test]
    fn test_contains_inside_and_outside_points() -> Result<()> {
        let airspace = germany()?;
        assert!(contains(&airspace, at(50.11, 8.68)?), "Frankfurt");
        assert!(contains(&airspace, at(52.52, 13.40)?), "Berlin");
        assert!(!contains(&airspace, at(48.86, 2.35)?), "Paris");
        assert!(!contains(&airspace, at(48.21, 16.37)?), "Vienna");
        Ok(())
    }

    #[test]
    fn test_crosses_real_overflights() -> Result<()> {
        let airspace = germany()?;
        let amsterdam = at(52.31, 4.76)?;
        let vienna = at(48.11, 16.57)?;
        let copenhagen = at(55.62, 12.65)?;
        let zurich = at(47.46, 8.55)?;
        let london = at(51.47, -0.45)?;
        let paris = at(49.01, 2.55)?;
        let madrid = at(40.47, -3.56)?;
        let barcelona = at(41.30, 2.08)?;
        assert!(crosses(&airspace, amsterdam, vienna));
        assert!(crosses(&airspace, copenhagen, zurich));
        assert!(!crosses(&airspace, london, paris));
        assert!(!crosses(&airspace, madrid, barcelona));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_crosses_is_symmetric(a in (40.0_f64..62.0, -8.0_f64..30.0), b in (40.0_f64..62.0, -8.0_f64..30.0)) {
            let fail = |e: anyhow::Error| TestCaseError::fail(e.to_string());
            let airspace = germany().map_err(fail)?;
            let a = at(a.0, a.1).map_err(fail)?;
            let b = at(b.0, b.1).map_err(fail)?;
            prop_assert_eq!(crosses(&airspace, a, b), crosses(&airspace, b, a));
        }

        #[test]
        fn test_route_from_inside_always_crosses(lat in 48.5_f64..53.0, lon in 8.0_f64..13.0, b in (40.0_f64..62.0, -8.0_f64..30.0)) {
            let fail = |e: anyhow::Error| TestCaseError::fail(e.to_string());
            let airspace = germany().map_err(fail)?;
            let inside = at(lat, lon).map_err(fail)?;
            prop_assume!(contains(&airspace, inside));
            prop_assert!(crosses(&airspace, inside, at(b.0, b.1).map_err(fail)?));
        }
    }
}
