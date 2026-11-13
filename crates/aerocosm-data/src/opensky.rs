//! The `OpenSky` flight lists (CC BY 4.0, 2019) aggregated into observed
//! traffic (plan M5 D97–D99): flights per airport, per airline and airport,
//! per airline and aircraft type, and coverage per country. Airline names and
//! callsigns come from `OpenFlights` (`ODbL`). Implausible flights are dropped.

use std::{
    collections::{BTreeMap, HashMap},
    hash::BuildHasher,
    io::Read,
};

pub use aerocosm_core::traffic::observed::ObservedTraffic;
use aerocosm_core::{
    LatLon,
    traffic::observed::{CountryCoverage, ObservedAirline, ObservedAirport},
    world::{
        WorldDefinition,
        codes::{AircraftTypeCode, AirlineIcao, AirportIdent, CountryCode},
    },
};
use anyhow::{Context, Result};
use serde::Deserialize;

/// No airliner covers ground faster than this, m/s (about 1,220 km/h, a jet
/// in a strong jet stream); faster flights joined two tracks.
const MAX_SPEED: f64 = 340.0;

/// Counts per key.
type Counts<K> = Vec<(K, u32)>;

/// One row of a flight list.
#[derive(Debug, Deserialize)]
struct Flight {
    callsign: String,
    typecode: String,
    origin: String,
    destination: String,
    firstseen: String,
    lastseen: String,
    day: String,
}

/// An airline's name and callsign (`OpenFlights`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirlineInfo {
    name: String,
    callsign: Option<String>,
}

/// Reads `OpenFlights`' `airlines.dat` (no header): name and callsign per ICAO
/// designator, an active entry winning over an inactive one.
pub fn read_openflights(reader: impl Read) -> Result<HashMap<AirlineIcao, AirlineInfo>> {
    let mut airlines: HashMap<AirlineIcao, (AirlineInfo, bool)> = HashMap::new();
    let mut rows = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);
    for (line, row) in rows.records().enumerate() {
        let row = row.with_context(|| format!("airlines.dat: cannot parse line {}", line + 1))?;
        let field = |k: usize| row.get(k).filter(|v| !v.is_empty() && *v != "\\N");
        let (Some(name), Some(Ok(icao))) = (field(1), field(4).map(str::parse::<AirlineIcao>))
        else {
            continue;
        };
        let active = field(7) == Some("Y");
        let info = AirlineInfo {
            name: name.to_owned(),
            callsign: field(5).map(str::to_owned),
        };
        match airlines.get(&icao) {
            Some((_, true)) => {}
            Some((_, false)) if !active => {}
            _ => {
                airlines.insert(icao, (info, active));
            }
        }
    }
    Ok(airlines
        .into_iter()
        .map(|(icao, (info, _))| (icao, info))
        .collect())
}

/// What the aggregation dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregateReport {
    /// Flights read.
    pub flights: u64,
    /// Flights dropped, by reason.
    pub rejected: BTreeMap<&'static str, u64>,
    /// Callsign prefixes with fewer flights than the minimum or no name.
    pub small_airlines: usize,
}

/// Counts flight lists into observed traffic.
#[derive(Debug)]
pub struct Aggregator {
    year: u16,
    airports: HashMap<String, (AirportIdent, LatLon, CountryCode)>,
    flights: HashMap<AirportIdent, (u32, u32)>,
    departures: HashMap<(AirlineIcao, AirportIdent), u32>,
    types: HashMap<(AirlineIcao, AircraftTypeCode), u32>,
    coverage: HashMap<CountryCode, (u32, u32)>,
    report: AggregateReport,
}

/// Seconds since 1970 of `YYYY-MM-DD HH:MM:SS…` in UTC.
fn seconds(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The airline of a callsign: three letters and then a digit.
fn airline_of(callsign: &str) -> Option<AirlineIcao> {
    let bytes = callsign.as_bytes();
    if bytes.len() > 3 && bytes[3].is_ascii_digit() {
        callsign.get(..3)?.parse().ok()
    } else {
        None
    }
}

impl Aggregator {
    /// An empty count for `year` over the airports of `world` with an ICAO
    /// code.
    pub fn new(world: &WorldDefinition, year: u16) -> Self {
        let airports = world
            .airports
            .iter()
            .filter_map(|a| Some((a.icao?.to_string(), (a.ident, a.location, a.country))))
            .collect();
        Self {
            year,
            airports,
            flights: HashMap::new(),
            departures: HashMap::new(),
            types: HashMap::new(),
            coverage: HashMap::new(),
            report: AggregateReport::default(),
        }
    }

    fn reject(&mut self, reason: &'static str) {
        *self.report.rejected.entry(reason).or_default() += 1;
    }

    /// Counts one flight list (CSV, decompressed).
    pub fn add(&mut self, reader: impl Read) -> Result<()> {
        for (line, row) in csv::Reader::from_reader(reader)
            .into_deserialize::<Flight>()
            .enumerate()
        {
            let flight =
                row.with_context(|| format!("flight list: cannot parse row {}", line + 1))?;
            self.count(&flight);
        }
        Ok(())
    }

    fn count(&mut self, flight: &Flight) {
        self.report.flights += 1;
        if flight.day.get(..4).and_then(|y| y.parse::<u16>().ok()) != Some(self.year) {
            self.reject("outside the year");
            return;
        }
        let origin = self.airports.get(&flight.origin).copied();
        let destination = self.airports.get(&flight.destination).copied();
        let (Some(origin), Some(destination)) = (origin, destination) else {
            if let Some((_, _, country)) = origin.or(destination) {
                self.coverage.entry(country).or_default().1 += 1;
            } else {
                self.reject("no known airport");
            }
            return;
        };
        if origin.0 == destination.0 {
            self.reject("same airport");
            return;
        }
        let duration = seconds(&flight.lastseen)
            .zip(seconds(&flight.firstseen))
            .map(|(last, first)| last - first);
        // SAFETY: flight times are seconds, far below 2^52.
        #[allow(clippy::cast_precision_loss)]
        let fast = duration.is_none_or(|d| {
            d <= 0 || origin.1.distance_to(destination.1).value() / d as f64 > MAX_SPEED
        });
        if fast {
            self.reject("faster than any airliner");
            return;
        }
        self.coverage.entry(origin.2).or_default().0 += 1;
        self.flights.entry(origin.0).or_default().0 += 1;
        self.flights.entry(destination.0).or_default().1 += 1;
        if let Some(airline) = airline_of(&flight.callsign) {
            *self.departures.entry((airline, origin.0)).or_default() += 1;
            if let Ok(code) = flight.typecode.parse::<AircraftTypeCode>() {
                *self.types.entry((airline, code)).or_default() += 1;
            }
        }
    }

    /// The observed traffic: airlines named in `airlines` with at least
    /// `min_flights` departures, everything sorted.
    pub fn finish<S: BuildHasher>(
        self,
        airlines: &HashMap<AirlineIcao, AirlineInfo, S>,
        min_flights: u32,
    ) -> (ObservedTraffic, AggregateReport) {
        let mut report = self.report;
        let mut airports: Vec<ObservedAirport> = self
            .flights
            .into_iter()
            .map(|(airport, (departures, arrivals))| ObservedAirport {
                airport,
                departures,
                arrivals,
            })
            .collect();
        airports.sort_by_key(|a| a.airport);
        let mut by_airline: BTreeMap<
            AirlineIcao,
            (Counts<AirportIdent>, Counts<AircraftTypeCode>),
        > = BTreeMap::new();
        for ((airline, airport), count) in self.departures {
            by_airline
                .entry(airline)
                .or_default()
                .0
                .push((airport, count));
        }
        for ((airline, code), count) in self.types {
            by_airline.entry(airline).or_default().1.push((code, count));
        }
        let mut observed_airlines = Vec::with_capacity(by_airline.len());
        for (icao, (mut departures, mut types)) in by_airline {
            let total: u32 = departures.iter().map(|(_, n)| n).sum();
            let Some(info) = airlines.get(&icao).filter(|_| total >= min_flights) else {
                report.small_airlines += 1;
                continue;
            };
            departures.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            types.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            observed_airlines.push(ObservedAirline {
                icao,
                name: info.name.clone(),
                callsign: info.callsign.clone(),
                departures,
                types,
            });
        }
        let mut coverage: Vec<CountryCoverage> = self
            .coverage
            .into_iter()
            .map(|(country, (complete, partial))| CountryCoverage {
                country,
                complete,
                partial,
            })
            .collect();
        coverage.sort_by_key(|c| c.country);
        (
            ObservedTraffic {
                year: self.year,
                airports,
                airlines: observed_airlines,
                coverage,
            },
            report,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use aerocosm_core::{
        LatLon,
        units::Feet,
        world::{AirportDefinition, AirportKind, WorldDefinition},
    };
    use anyhow::{Context, Result};

    use super::*;

    fn airport(icao: &str, lat: f64, lon: f64, country: &str) -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: icao.parse()?,
            icao: Some(icao.parse()?),
            iata: None,
            name: icao.to_owned(),
            location: LatLon::new(lat, lon)?,
            elevation: Feet::new(0.0)?,
            kind: AirportKind::Major,
            country: country.parse()?,
            catchment: None,
            scheduled: true,
            zone: None,
            wind: None,
        })
    }

    fn world() -> Result<WorldDefinition> {
        Ok(WorldDefinition {
            format_version: aerocosm_core::world::FORMAT_VERSION,
            airports: vec![
                airport("EDDF", 50.03, 8.57, "DE")?,
                airport("EDDM", 48.35, 11.79, "DE")?,
                airport("YMML", -37.67, 144.84, "AU")?,
            ],
            runways: Vec::new(),
        })
    }

    const HEADER: &str = "callsign,number,aircraft_uid,typecode,origin,destination,firstseen,\
                          lastseen,day,latitude_1,longitude_1,altitude_1,latitude_2,longitude_2,\
                          altitude_2";

    fn flights() -> String {
        [
            HEADER,
            // Two Lufthansa flights FRA-MUC, 55 minutes for 300 km.
            "DLH100,,a,A320,EDDF,EDDM,2019-03-01 07:00:00+00:00,2019-03-01 \
             07:55:00+00:00,2019-03-01 00:00:00+00:00,,,,,,",
            "DLH102,,b,,EDDF,EDDM,2019-03-01 09:00:00+00:00,2019-03-01 09:55:00+00:00,2019-03-01 \
             00:00:00+00:00,,,,,,",
            // Back MUC-FRA.
            "DLH101,,a,A320,EDDM,EDDF,2019-03-01 08:30:00+00:00,2019-03-01 \
             09:20:00+00:00,2019-03-01 00:00:00+00:00,,,,,,",
            // Melbourne to Frankfurt in four hours: impossible.
            "HVN19,,c,,YMML,EDDF,2019-03-01 00:43:16+00:00,2019-03-01 04:56:29+00:00,2019-03-01 \
             00:00:00+00:00,,,,,,",
            // Same airport.
            ",,d,A332,EDDF,EDDF,2019-03-01 01:07:21+00:00,2019-03-01 03:32:59+00:00,2019-03-01 \
             00:00:00+00:00,,,,,,",
            // Destination not seen: coverage only.
            "DLH200,,e,,EDDF,,2019-03-01 10:00:00+00:00,2019-03-01 10:30:00+00:00,2019-03-01 \
             00:00:00+00:00,,,,,,",
            // Unknown airport code: coverage only.
            "XYZ1,,f,,EDDF,ZZZZ,2019-03-01 10:00:00+00:00,2019-03-01 11:30:00+00:00,2019-03-01 \
             00:00:00+00:00,,,,,,",
            // A private registration, no airline.
            "DABCD,,g,C172,EDDM,EDDF,2019-03-02 10:00:00+00:00,2019-03-02 \
             11:30:00+00:00,2019-03-02 00:00:00+00:00,,,,,,",
            // The previous year's last day, in January's file: not 2019.
            "DLH999,,h,,EDDF,EDDM,2018-12-31 22:00:00+00:00,2018-12-31 22:55:00+00:00,2018-12-31 \
             00:00:00+00:00,,,,,,",
        ]
        .join("\n")
    }

    const AIRLINES: &str = "3320,\"Lufthansa\",\\N,\"LH\",\"DLH\",\"LUFTHANSA\",\"Germany\",\"Y\"\\
                            \
                            n1,\"Old \
                            Lufthansa\",\\N,\"\",\"DLH\",\"OLD\",\"Germany\",\"N\"\n2,\"Private \
                            flight\",\\N,\"-\",\"N/A\",\"\",\"\",\"Y\"\n";

    fn aggregate(min_flights: u32) -> Result<(ObservedTraffic, AggregateReport)> {
        let mut aggregator = Aggregator::new(&world()?, 2019);
        aggregator.add(Cursor::new(flights()))?;
        let airlines = read_openflights(Cursor::new(AIRLINES))?;
        Ok(aggregator.finish(&airlines, min_flights))
    }

    #[test]
    fn test_flights_count_per_airport_and_airline() -> Result<()> {
        let (observed, report) = aggregate(1)?;
        let airport = |ident: &str| {
            observed
                .airports
                .iter()
                .find(|a| a.airport.as_str() == ident)
                .map(|a| (a.departures, a.arrivals))
        };
        // The private flight counts at the airports, not for an airline.
        assert_eq!(airport("EDDF"), Some((2, 2)));
        assert_eq!(airport("EDDM"), Some((2, 2)));
        assert_eq!(airport("YMML"), None);
        let dlh = observed.airlines.first().context("Lufthansa")?;
        assert_eq!(observed.airlines.len(), 1);
        assert_eq!(dlh.name, "Lufthansa");
        assert_eq!(dlh.callsign.as_deref(), Some("LUFTHANSA"));
        assert_eq!(dlh.departures, [("EDDF".parse()?, 2), ("EDDM".parse()?, 1)]);
        assert_eq!(dlh.types, [("A320".parse()?, 2)]);
        assert_eq!(report.rejected.get("faster than any airliner"), Some(&1));
        assert_eq!(report.rejected.get("same airport"), Some(&1));
        assert_eq!(report.rejected.get("outside the year"), Some(&1));
        Ok(())
    }

    #[test]
    fn test_coverage_counts_flights_with_one_end_missing() -> Result<()> {
        let (observed, _) = aggregate(1)?;
        let germany = observed
            .coverage
            .iter()
            .find(|c| c.country.as_str() == "DE")
            .context("Germany")?;
        // Four complete German departures; two seen without an arrival.
        assert_eq!((germany.complete, germany.partial), (4, 2));
        Ok(())
    }

    #[test]
    fn test_small_or_unnamed_operators_are_not_airlines() -> Result<()> {
        let (observed, report) = aggregate(4)?;
        assert_eq!(observed.airlines.len(), 0);
        assert_eq!(report.small_airlines, 1);
        Ok(())
    }

    #[test]
    fn test_timestamps_and_callsigns() {
        assert_eq!(seconds("1970-01-01 00:00:00+00:00"), Some(0));
        assert_eq!(seconds("2019-03-01 07:00:00+00:00"), Some(1_551_423_600));
        assert_eq!(seconds("2020-02-29 23:59:59"), Some(1_583_020_799));
        assert_eq!(seconds("garbage"), None);
        assert_eq!(
            airline_of("DLH4AB").map(|a| a.to_string()).as_deref(),
            Some("DLH")
        );
        assert_eq!(airline_of("DABCD"), None);
        assert_eq!(airline_of("DLH"), None);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1000))]

        #[test]
        fn test_seconds_count_whole_days(days in 0_i64..800, second in 0_i64..86_400) {
            // Day by day from 1970 on: one day later is 86,400 s later.
            let start = seconds("1970-01-01 00:00:00").unwrap_or(-1);
            proptest::prop_assert_eq!(start, 0);
            let mut date = (1970_i64, 1_i64, 1_i64);
            let mut left = days;
            let lengths = |y: i64| [31, if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            while left > 0 {
                let month_length = lengths(date.0)[usize::try_from(date.1 - 1).unwrap_or(0)];
                if date.2 < month_length {
                    date.2 += 1;
                } else if date.1 < 12 {
                    date = (date.0, date.1 + 1, 1);
                } else {
                    date = (date.0 + 1, 1, 1);
                }
                left -= 1;
            }
            let text = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                date.0, date.1, date.2, second / 3600, second / 60 % 60, second % 60
            );
            proptest::prop_assert_eq!(seconds(&text), Some(days * 86_400 + second));
        }
    }
}
