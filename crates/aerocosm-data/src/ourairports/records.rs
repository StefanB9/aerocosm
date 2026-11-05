//! Typed rows of the `OurAirports` CSV files. Unlisted columns are ignored.

use serde::Deserialize;

/// A row of `airports.csv`.
#[derive(Debug, Deserialize)]
pub(super) struct AirportRecord {
    pub(super) ident: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) latitude_deg: f64,
    pub(super) longitude_deg: f64,
    pub(super) elevation_ft: Option<f64>,
    pub(super) iso_country: String,
    pub(super) scheduled_service: String,
    pub(super) icao_code: Option<String>,
    pub(super) iata_code: Option<String>,
}

/// A row of `runways.csv`; `le` is the lower-numbered end, `he` the higher.
#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub(super) struct RunwayRecord {
    pub(super) airport_ident: String,
    pub(super) length_ft: Option<f64>,
    pub(super) width_ft: Option<f64>,
    pub(super) surface: Option<String>,
    pub(super) closed: Option<String>,
    pub(super) le_ident: Option<String>,
    pub(super) le_latitude_deg: Option<f64>,
    pub(super) le_longitude_deg: Option<f64>,
    pub(super) le_heading_degT: Option<f64>,
    pub(super) le_displaced_threshold_ft: Option<f64>,
    pub(super) he_ident: Option<String>,
    pub(super) he_latitude_deg: Option<f64>,
    pub(super) he_longitude_deg: Option<f64>,
    pub(super) he_heading_degT: Option<f64>,
    pub(super) he_displaced_threshold_ft: Option<f64>,
}
