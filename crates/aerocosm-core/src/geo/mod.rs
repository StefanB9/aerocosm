//! Geographic positions on a spherical Earth.

mod great_circle;
mod polyline;
mod route;
mod segment;

use std::fmt;

use anyhow::Result;
pub use great_circle::EARTH_RADIUS;
pub(crate) use polyline::Polyline;
pub(crate) use route::Route;
pub(crate) use segment::Segment;
use serde::{Deserialize, Serialize};

use crate::{CoreError, units::Degrees};

/// A validated geographic position in decimal degrees (WGS-84 datum, spherical
/// maths).
///
/// Latitude is within `[-90, 90]` and longitude within `[-180, 180]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawLatLon")]
pub struct LatLon {
    lat: f64,
    lon: f64,
}

/// Unvalidated wire form of [`LatLon`].
#[derive(Deserialize)]
struct RawLatLon {
    lat: f64,
    lon: f64,
}

impl TryFrom<RawLatLon> for LatLon {
    type Error = anyhow::Error;

    fn try_from(raw: RawLatLon) -> Result<Self> {
        Self::new(raw.lat, raw.lon)
    }
}

impl LatLon {
    /// Validates and builds a position from latitude and longitude in degrees.
    pub fn new(lat: f64, lon: f64) -> Result<Self> {
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
            Ok(Self { lat, lon })
        } else {
            Err(CoreError::InvalidCoordinate { lat, lon }.into())
        }
    }

    /// Builds a position from radians already known to be in range.
    pub(crate) fn from_radians_unchecked(lat: f64, lon: f64) -> Self {
        Self {
            lat: lat.to_degrees().clamp(-90.0, 90.0),
            lon: normalize_longitude(lon.to_degrees()),
        }
    }

    /// Latitude, positive north.
    pub const fn latitude(self) -> Degrees {
        Degrees::from_raw(self.lat)
    }

    /// Longitude, positive east.
    pub const fn longitude(self) -> Degrees {
        Degrees::from_raw(self.lon)
    }

    /// Latitude and longitude in radians.
    pub(crate) const fn to_radians(self) -> (f64, f64) {
        (self.lat.to_radians(), self.lon.to_radians())
    }
}

/// Wraps a longitude in degrees into `[-180, 180)`.
fn normalize_longitude(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    // `rem_euclid` can round up to exactly 360.0 for tiny negative inputs.
    if wrapped >= 180.0 { -180.0 } else { wrapped }
}

impl fmt::Display for LatLon {
    /// Formats as e.g. `50.033333°N 8.570556°E`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ns = if self.lat < 0.0 { 'S' } else { 'N' };
        let ew = if self.lon < 0.0 { 'W' } else { 'E' };
        write!(f, "{:.6}°{ns} {:.6}°{ew}", self.lat.abs(), self.lon.abs())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::CoreError;

    #[test]
    fn test_lat_lon_new_accepts_bounds() -> Result<()> {
        let point = LatLon::new(-90.0, 180.0)?;
        assert!((point.latitude().value() + 90.0).abs() < f64::EPSILON);
        assert!((point.longitude().value() - 180.0).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn test_lat_lon_new_rejects_out_of_range() -> Result<()> {
        for (lat, lon) in [
            (90.1, 0.0),
            (-91.0, 0.0),
            (0.0, 180.5),
            (0.0, -181.0),
            (f64::NAN, 0.0),
        ] {
            let error = LatLon::new(lat, lon).err().context("must reject")?;
            assert!(matches!(
                error.downcast_ref::<CoreError>(),
                Some(CoreError::InvalidCoordinate { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn test_lat_lon_serde_validates() -> Result<()> {
        let point = LatLon::new(50.033_333, 8.570_556)?;
        let json = serde_json::to_string(&point).context("serialise")?;
        assert_eq!(json, r#"{"lat":50.033333,"lon":8.570556}"#);
        let back: LatLon = serde_json::from_str(&json).context("deserialise")?;
        assert_eq!(back, point);
        assert!(serde_json::from_str::<LatLon>(r#"{"lat":95.0,"lon":0.0}"#).is_err());
        Ok(())
    }

    #[test]
    fn test_lat_lon_display() -> Result<()> {
        assert_eq!(
            LatLon::new(50.5, -8.25)?.to_string(),
            "50.500000°N 8.250000°W"
        );
        Ok(())
    }
}
