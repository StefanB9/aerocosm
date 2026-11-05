//! Converting `OurAirports` airport rows.

use aerocosm_core::{
    LatLon,
    units::Feet,
    world::{
        AirportDefinition, AirportKind,
        codes::{AirportIdent, CountryCode, IcaoCode},
    },
};
use anyhow::Result;

use super::{Fallbacks, records::AirportRecord};

/// Converts a row, or `None` if it is not an aerodrome in `country` with valid
/// data.
pub(super) fn convert_airport(
    record: &AirportRecord,
    country: CountryCode,
    edges: &[IcaoCode],
    fallbacks: &mut Fallbacks,
) -> Result<Option<AirportDefinition>> {
    let listed_edge = record
        .icao_code
        .as_deref()
        .or(Some(record.ident.as_str()))
        .and_then(|code| code.parse::<IcaoCode>().ok())
        .is_some_and(|icao| edges.contains(&icao));
    let kind = match (record.iso_country == country.as_str(), record.kind.as_str()) {
        (true, "large_airport") => AirportKind::Major,
        (true, "medium_airport") => AirportKind::Regional,
        (true, "small_airport") if record.scheduled_service == "yes" => AirportKind::Regional,
        (true, "small_airport") => AirportKind::Airfield,
        (false, "large_airport" | "medium_airport") if listed_edge => AirportKind::Edge,
        _ => return Ok(None),
    };
    let Ok(country) = record.iso_country.parse::<CountryCode>() else {
        return Ok(None);
    };
    // D22: the ICAO code is the ident whenever there is one.
    let icao = record
        .icao_code
        .as_deref()
        .and_then(|code| code.parse::<IcaoCode>().ok())
        .or_else(|| record.ident.parse::<IcaoCode>().ok());
    let ident = match icao {
        Some(icao) => AirportIdent::from(icao),
        None => match record.ident.parse() {
            Ok(ident) => ident,
            Err(_) => return Ok(None),
        },
    };
    let name = record.name.trim();
    let Ok(location) = LatLon::new(record.latitude_deg, record.longitude_deg) else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    let elevation = if let Some(feet) = record.elevation_ft {
        Feet::new(feet)?
    } else {
        fallbacks.count("missing elevation");
        Feet::new(0.0)?
    };
    Ok(Some(AirportDefinition {
        ident,
        icao,
        iata: record
            .iata_code
            .as_deref()
            .and_then(|code| code.parse().ok()),
        name: name.to_owned(),
        location,
        elevation,
        kind,
        country,
        catchment: None,
    }))
}
