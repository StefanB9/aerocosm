//! Time zones per airport (plan M5 D101) from the `airportsdata` table (MIT;
//! its time zones come from TimeZoneDB): by ICAO code, else by IATA code,
//! else the zone of the nearest airport in the same country that has one.

use std::{collections::HashMap, io::Read};

use aerocosm_core::{
    LatLon,
    time::Zone,
    world::{AirportDefinition, AirportKind},
};
use anyhow::{Context, Result};
use serde::Deserialize;

/// One airport of the table with a valid zone.
#[derive(Debug, Clone)]
pub struct ZoneRecord {
    icao: Option<String>,
    iata: Option<String>,
    country: String,
    location: LatLon,
    zone: Zone,
}

/// A raw row of the table.
#[derive(Debug, Deserialize)]
struct Row {
    icao: String,
    iata: String,
    country: String,
    lat: f64,
    lon: f64,
    tz: String,
}

/// Reads the `airportsdata` CSV, keeping rows whose zone the bundled IANA
/// database knows.
pub fn read_zones(reader: impl Read) -> Result<Vec<ZoneRecord>> {
    let mut records = Vec::new();
    for (line, row) in csv::Reader::from_reader(reader)
        .into_deserialize::<Row>()
        .enumerate()
    {
        let row = row.with_context(|| format!("airportsdata: cannot parse record {}", line + 1))?;
        let (Ok(zone), Ok(location)) = (Zone::new(&row.tz), LatLon::new(row.lat, row.lon)) else {
            continue;
        };
        let code = |text: String| Some(text).filter(|t| !t.is_empty());
        records.push(ZoneRecord {
            icao: code(row.icao),
            iata: code(row.iata),
            country: row.country,
            location,
            zone,
        });
    }
    Ok(records)
}

/// How the zones were found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZoneReport {
    /// By ICAO or IATA code.
    pub by_code: usize,
    /// From the nearest listed airport in the same country.
    pub by_neighbour: usize,
    /// Without a zone.
    pub missing: usize,
}

/// Gives every airport that can have traffic (scheduled, or not a mere
/// airfield) its zone.
pub fn assign_zones(airports: &mut [AirportDefinition], records: &[ZoneRecord]) -> ZoneReport {
    let mut by_icao: HashMap<&str, usize> = HashMap::with_capacity(records.len());
    let mut by_iata: HashMap<&str, usize> = HashMap::with_capacity(records.len());
    let mut by_country: HashMap<&str, Vec<usize>> = HashMap::new();
    for (k, record) in records.iter().enumerate() {
        if let Some(icao) = &record.icao {
            by_icao.insert(icao, k);
        }
        if let Some(iata) = &record.iata {
            by_iata.insert(iata, k);
        }
        by_country.entry(&record.country).or_default().push(k);
    }
    let mut report = ZoneReport::default();
    for airport in airports
        .iter_mut()
        .filter(|a| a.scheduled || a.kind != AirportKind::Airfield)
    {
        let by_code = airport
            .icao
            .and_then(|icao| by_icao.get(icao.as_str()))
            .or_else(|| airport.iata.and_then(|iata| by_iata.get(iata.as_str())));
        if let Some(&k) = by_code {
            airport.zone = Some(records[k].zone.clone());
            report.by_code += 1;
            continue;
        }
        let nearest = by_country
            .get(airport.country.as_str())
            .into_iter()
            .flatten()
            .min_by(|&&a, &&b| {
                let distance = |k: usize| records[k].location.distance_to(airport.location).value();
                distance(a).total_cmp(&distance(b))
            });
        if let Some(&k) = nearest {
            airport.zone = Some(records[k].zone.clone());
            report.by_neighbour += 1;
        } else {
            report.missing += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use aerocosm_core::{
        LatLon,
        units::Feet,
        world::{AirportDefinition, AirportKind},
    };
    use anyhow::{Context, Result};

    use super::*;

    const TABLE: &str = "\"icao\",\"iata\",\"name\",\"city\",\"subd\",\"country\",\"elevation\",\"\
                         lat\",\"lon\",\"tz\",\"lid\"
\"EDDF\",\"FRA\",\"Frankfurt\",\"Frankfurt\",\"Hesse\",\"DE\",364,50.033333,8.570556,\"Europe/\
                         Berlin\",\"\"
\"KJFK\",\"JFK\",\"John F Kennedy\",\"New York\",\"New \
                         York\",\"US\",13,40.639751,-73.778925,\"America/New_York\",\"JFK\"
\"KLAX\",\"LAX\",\"Los Angeles\",\"Los \
                         Angeles\",\"California\",\"US\",125,33.942536,-118.408075,\"America/\
                         Los_Angeles\",\"LAX\"
\"XXXX\",\"\",\"Broken\",\"\",\"\",\"DE\",0,50.0,8.0,\"Not/AZone\",\"\"
";

    fn airport(
        ident: &str,
        iata: Option<&str>,
        lat: f64,
        lon: f64,
        country: &str,
    ) -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: ident.parse()?,
            icao: ident.parse().ok(),
            iata: iata.map(str::parse).transpose()?,
            name: ident.to_owned(),
            location: LatLon::new(lat, lon)?,
            elevation: Feet::new(0.0)?,
            kind: AirportKind::Regional,
            country: country.parse()?,
            catchment: None,
            scheduled: true,
            zone: None,
            wind: None,
        })
    }

    fn zone_of(airports: &[AirportDefinition], ident: &str) -> Result<Option<String>> {
        Ok(airports
            .iter()
            .find(|a| a.ident.as_str() == ident)
            .context("airport")?
            .zone
            .as_ref()
            .map(|zone| zone.name().to_owned()))
    }

    #[test]
    fn test_zones_by_code_then_by_neighbour() -> Result<()> {
        let mut airports = vec![
            airport("EDDF", Some("FRA"), 50.03, 8.57, "DE")?,
            // Listed only by IATA code under another ICAO code.
            airport("KXJF", Some("JFK"), 40.64, -73.78, "US")?,
            // Not listed: nearest US airport with a zone is Los Angeles.
            airport("US-0001", None, 34.0, -118.0, "US")?,
            // Not listed and no neighbour in its country.
            airport("FR-0001", None, 48.0, 2.0, "FR")?,
        ];
        let report = assign_zones(&mut airports, &read_zones(Cursor::new(TABLE))?);
        assert_eq!(
            zone_of(&airports, "EDDF")?.as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            zone_of(&airports, "KXJF")?.as_deref(),
            Some("America/New_York")
        );
        assert_eq!(
            zone_of(&airports, "US-0001")?.as_deref(),
            Some("America/Los_Angeles")
        );
        assert_eq!(zone_of(&airports, "FR-0001")?, None);
        assert_eq!(
            (report.by_code, report.by_neighbour, report.missing),
            (2, 1, 1)
        );
        Ok(())
    }

    #[test]
    fn test_unknown_zone_names_are_dropped() -> Result<()> {
        let records = read_zones(Cursor::new(TABLE))?;
        assert!(records.iter().all(|r| r.icao.as_deref() != Some("XXXX")));
        assert_eq!(records.len(), 3);
        Ok(())
    }
}
