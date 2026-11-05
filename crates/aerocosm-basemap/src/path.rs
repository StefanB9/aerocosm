//! A sequence of positions, serialised as `[[lon, lat], ...]`.

use aerocosm_core::LatLon;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeSeq};

/// An ordered sequence of positions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path(Vec<LatLon>);

impl Path {
    /// Wraps positions in order.
    pub const fn new(points: Vec<LatLon>) -> Self {
        Self(points)
    }

    /// The positions in order.
    pub fn points(&self) -> &[LatLon] {
        &self.0
    }
}

impl Serialize for Path {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for point in &self.0 {
            seq.serialize_element(&[point.longitude().value(), point.latitude().value()])?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pairs = Vec::<[f64; 2]>::deserialize(deserializer)?;
        let mut points = Vec::with_capacity(pairs.len());
        for [lon, lat] in pairs {
            points.push(LatLon::new(lat, lon).map_err(serde::de::Error::custom)?);
        }
        Ok(Self(points))
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_path_serialises_as_lon_lat_pairs() -> Result<()> {
        let path = Path::new(vec![LatLon::new(50.0, 8.5)?, LatLon::new(-1.25, 179.0)?]);
        let json = serde_json::to_string(&path).context("serialise")?;
        assert_eq!(json, "[[8.5,50.0],[179.0,-1.25]]");
        assert_eq!(
            serde_json::from_str::<Path>(&json).context("deserialise")?,
            path
        );
        Ok(())
    }

    #[test]
    fn test_path_deserialise_rejects_invalid_coordinates() {
        assert!(serde_json::from_str::<Path>("[[8.5,95.0]]").is_err());
        assert!(serde_json::from_str::<Path>("[[8.5]]").is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_path_json_round_trip(points in prop::collection::vec((-90.0_f64..=90.0, -180.0_f64..=180.0), 0..50)) {
            let mut path = Vec::with_capacity(points.len());
            for (lat, lon) in points {
                path.push(LatLon::new(lat, lon).map_err(|e| TestCaseError::fail(e.to_string()))?);
            }
            let path = Path::new(path);
            let json = serde_json::to_string(&path).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let back: Path = serde_json::from_str(&json).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(back, path);
        }
    }
}
