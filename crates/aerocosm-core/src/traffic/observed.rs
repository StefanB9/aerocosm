//! Observed traffic (plan M5 D97–D99): what the `OpenSky` flight lists show for
//! the reference year, aggregated by the importer. The world model calibrates
//! airports to it and derives airlines from it; nothing here is curated.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    asset::{self, AssetKind},
    world::codes::{AircraftTypeCode, AirlineIcao, AirportIdent, CountryCode},
};

/// Observed traffic as a binary asset.
const OBSERVED_ASSET: AssetKind = AssetKind {
    tag: *b"OBSV",
    name: "observed traffic asset",
    version: 2,
};

/// The traffic of one year as the `OpenSky` Network saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedTraffic {
    /// The reference year.
    pub year: u16,
    /// Flights per airport, sorted by airport.
    pub airports: Vec<ObservedAirport>,
    /// Airlines seen, sorted by ICAO designator.
    pub airlines: Vec<ObservedAirline>,
    /// How completely flights were seen, per country of departure.
    pub coverage: Vec<CountryCoverage>,
}

/// Flights seen leaving and reaching one airport in the year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedAirport {
    /// The airport.
    pub airport: AirportIdent,
    /// Departures seen.
    pub departures: u32,
    /// Arrivals seen.
    pub arrivals: u32,
    /// Flights seen at the airport whose other end was not seen (version 2):
    /// with the complete ones, how many flights the airport's receivers saw.
    pub partial: u32,
}

/// An airline seen in the year: who it is and where and with what it flew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedAirline {
    /// ICAO designator, the callsign prefix.
    pub icao: AirlineIcao,
    /// Name (`OpenFlights`).
    pub name: String,
    /// Radio callsign (`OpenFlights`), where known.
    pub callsign: Option<String>,
    /// Departures per airport, most first.
    pub departures: Vec<(AirportIdent, u32)>,
    /// Flights per aircraft type, where the type was known, most first.
    pub types: Vec<(AircraftTypeCode, u32)>,
}

/// Flights departing one country: seen with both ends (`complete`) or with
/// one end missing (`partial`), a measure of receiver coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountryCoverage {
    /// The country.
    pub country: CountryCode,
    /// Flights seen from departure to arrival.
    pub complete: u32,
    /// Flights whose departure or arrival was not seen.
    pub partial: u32,
}

impl CountryCoverage {
    /// The share of flights seen with both ends, 0 when none were seen.
    pub fn share(&self) -> f64 {
        let all = f64::from(self.complete) + f64::from(self.partial);
        if all > 0.0 {
            f64::from(self.complete) / all
        } else {
            0.0
        }
    }
}

impl ObservedTraffic {
    /// As a binary asset (plan M5 D104).
    pub fn to_asset(&self) -> Result<Vec<u8>> {
        asset::encode(OBSERVED_ASSET, self)
    }

    /// Reads an asset written by [`Self::to_asset`].
    pub fn from_asset(bytes: &[u8]) -> Result<Self> {
        asset::decode(OBSERVED_ASSET, bytes)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn sample() -> Result<ObservedTraffic> {
        Ok(ObservedTraffic {
            year: 2019,
            airports: vec![ObservedAirport {
                airport: "EDDF".parse()?,
                departures: 230_000,
                arrivals: 229_000,
                partial: 140_000,
            }],
            airlines: vec![ObservedAirline {
                icao: "DLH".parse()?,
                name: "Lufthansa".to_owned(),
                callsign: Some("LUFTHANSA".to_owned()),
                departures: vec![("EDDF".parse()?, 150_000), ("EDDM".parse()?, 120_000)],
                types: vec![("A320".parse()?, 60_000), ("B748".parse()?, 5_000)],
            }],
            coverage: vec![CountryCoverage {
                country: "DE".parse()?,
                complete: 900_000,
                partial: 100_000,
            }],
        })
    }

    #[test]
    fn test_observed_traffic_round_trips_as_an_asset() -> Result<()> {
        let observed = sample()?;
        assert_eq!(
            ObservedTraffic::from_asset(&observed.to_asset()?)?,
            observed
        );
        Ok(())
    }

    #[test]
    fn test_coverage_is_the_share_of_flights_with_both_ends() -> Result<()> {
        let observed = sample()?;
        let share = observed.coverage[0].share();
        assert!((share - 0.9).abs() < 1e-12);
        let none = CountryCoverage {
            country: "XX".parse()?,
            complete: 0,
            partial: 0,
        };
        assert!(none.share().abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn test_shipped_observed_traffic_shows_the_real_network() -> Result<()> {
        use anyhow::Context;
        let observed =
            ObservedTraffic::from_asset(include_bytes!("../../../../assets/traffic/observed.bin"))?;
        assert_eq!(observed.year, 2019);
        assert!(
            observed.airports.len() > 4_000,
            "{}",
            observed.airports.len()
        );
        assert!(observed.airlines.len() > 400, "{}", observed.airlines.len());
        let frankfurt = observed
            .airports
            .iter()
            .find(|a| a.airport.as_str() == "EDDF")
            .context("Frankfurt")?;
        // About 257,000 departures in 2019, about 60 % of them seen whole.
        assert!(
            (120_000..200_000).contains(&frankfurt.departures),
            "{frankfurt:?}"
        );
        assert!(frankfurt.partial > 50_000, "{frankfurt:?}");
        // Tegel's traffic, closed since, counts at BER.
        let berlin = observed
            .airports
            .iter()
            .find(|a| a.airport.as_str() == "EDDB")
            .context("BER")?;
        assert!(berlin.departures > 80_000, "{berlin:?}");
        let lufthansa = observed
            .airlines
            .iter()
            .find(|a| a.icao.as_str() == "DLH")
            .context("Lufthansa")?;
        assert_eq!(
            lufthansa.departures.first().map(|d| d.0.as_str()),
            Some("EDDF")
        );
        assert_eq!(lufthansa.name, "Lufthansa");
        let share = |code: &str| {
            observed
                .coverage
                .iter()
                .find(|c| c.country.as_str() == code)
                .map(CountryCoverage::share)
        };
        assert!(share("DE").is_some_and(|s| s > 0.5));
        assert!(share("US").is_some_and(|s| s > 0.4));
        Ok(())
    }
}
