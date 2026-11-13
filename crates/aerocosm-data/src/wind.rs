//! Prevailing wind per scheduled airport (plan M5 D102) from the NASA POWER
//! climatology (CC BY 4.0): the long-term annual mean direction of the wind
//! at 10 m, one request per airport. The importer only writes the requests
//! and reads the downloaded responses; it fetches nothing itself.

use std::{collections::HashMap, hash::BuildHasher};

use aerocosm_core::{
    LatLon,
    units::Degrees,
    world::{AirportDefinition, codes::AirportIdent},
};
use anyhow::{Context, Result};
use serde_json::Value;

/// One `IDENT<TAB>URL` line per scheduled airport: the requests to download,
/// each response saved as `<IDENT>.json`.
pub fn wind_requests(airports: &[AirportDefinition]) -> String {
    let mut lines = String::with_capacity(airports.len() * 160);
    for airport in airports.iter().filter(|a| a.scheduled) {
        lines.push_str(airport.ident.as_str());
        lines.push('\t');
        lines.push_str(&wind_url(airport.location));
        lines.push('\n');
    }
    lines
}

/// Gives every airport in `winds` its prevailing wind; returns how many got
/// one.
pub fn assign_winds<S: BuildHasher>(
    airports: &mut [AirportDefinition],
    winds: &HashMap<AirportIdent, Degrees, S>,
) -> usize {
    let mut assigned = 0;
    for airport in airports {
        if let Some(&wind) = winds.get(&airport.ident) {
            airport.wind = Some(wind);
            assigned += 1;
        }
    }
    assigned
}

/// POWER's marker for a missing value.
const MISSING: f64 = -999.0;

/// The climatology request for the wind at `location`.
pub fn wind_url(location: LatLon) -> String {
    format!(
        "https://power.larc.nasa.gov/api/temporal/climatology/point?parameters=WD10M\
         &community=RE&longitude={:.4}&latitude={:.4}&format=JSON",
        location.longitude().value(),
        location.latitude().value()
    )
}

/// The annual mean direction the wind blows from, in a POWER response;
/// `None` where POWER has no value.
pub fn parse_wind(json: &str) -> Result<Option<Degrees>> {
    let value: Value = serde_json::from_str(json).context("malformed POWER response")?;
    let annual = value
        .pointer("/properties/parameter/WD10M/ANN")
        .and_then(Value::as_f64)
        .context("POWER response without WD10M/ANN")?;
    if (annual - MISSING).abs() < 1e-9 {
        return Ok(None);
    }
    Ok(Some(Degrees::new(annual)?.normalized()))
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::{Context, Result};

    use super::*;

    const FRANKFURT: &str = r#"{"type": "Feature", "properties": {"parameter": {"WD10M": {"JAN": 221.8, "JUL": 267.3, "ANN": 232.3}}}}"#;

    #[test]
    fn test_the_annual_direction_is_read() -> Result<()> {
        let wind = parse_wind(FRANKFURT)?.context("a direction")?;
        assert!((wind.value() - 232.3).abs() < 1e-9);
        // POWER marks missing values with -999.
        let missing = FRANKFURT.replace("232.3", "-999");
        assert_eq!(parse_wind(&missing)?, None);
        assert!(parse_wind("not json").is_err());
        Ok(())
    }

    #[test]
    fn test_requests_name_the_point() -> Result<()> {
        let url = wind_url(LatLon::new(50.033_333, 8.570_556)?);
        assert_eq!(
            url,
            "https://power.larc.nasa.gov/api/temporal/climatology/point?parameters=WD10M\
             &community=RE&longitude=8.5706&latitude=50.0333&format=JSON"
        );
        Ok(())
    }

    #[test]
    fn test_requests_for_scheduled_airports_only() -> Result<()> {
        use aerocosm_core::{
            units::Feet,
            world::{AirportDefinition, AirportKind},
        };
        let airport = |ident: &str, scheduled: bool| -> Result<AirportDefinition> {
            Ok(AirportDefinition {
                ident: ident.parse()?,
                icao: None,
                iata: None,
                name: ident.to_owned(),
                location: LatLon::new(50.0, 8.0)?,
                elevation: Feet::new(0.0)?,
                kind: AirportKind::Regional,
                country: "DE".parse()?,
                catchment: None,
                scheduled,
                zone: None,
                wind: None,
            })
        };
        let lines = wind_requests(&[airport("EDDF", true)?, airport("EDFE", false)?]);
        assert_eq!(lines.lines().count(), 1);
        assert!(
            lines.starts_with("EDDF\thttps://power.larc.nasa.gov/"),
            "{lines}"
        );
        Ok(())
    }
}
