//! `OurAirports` CSV → world definition.

mod airport;
mod magnetic;
mod records;
mod runway;
mod surface;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Read,
};

use aerocosm_core::{
    LatLon,
    units::Degrees,
    world::{
        AirportDefinition, AirportKind, FORMAT_VERSION, GeometrySource, WorldDefinition,
        codes::{AirportIdent, CountryCode, IcaoCode},
    },
};
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

use self::{
    airport::convert_airport,
    magnetic::{DesignatorOffsets, offset_of},
    records::{AirportRecord, RunwayRecord},
    runway::convert_runway,
};
use crate::natural_earth::PopulatedPlace;

/// Readers for `airports.csv` and `runways.csv`.
#[derive(Debug)]
pub struct OurAirportsSources<R> {
    /// `airports.csv`.
    pub airports: R,
    /// `runways.csv`.
    pub runways: R,
}

/// Which country to import and how to turn magnetic designators into true
/// headings.
#[derive(Debug, Clone)]
pub struct WorldSettings {
    /// The country, or `None` for the whole globe.
    country: Option<CountryCode>,
    /// Foreign airports imported as edge airports.
    edges: Vec<IcaoCode>,
    /// How synthesised runway headings follow from designators.
    declination: Declination,
}

/// How a runway designator turns into a true heading.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Declination {
    /// One magnetic declination everywhere, positive east.
    Fixed(Degrees),
    /// The offset learnt from the nearest surveyed runways (plan M5 D101).
    Learnt,
}

/// Foreign airports imported as edge airports (plan D29): major European
/// destinations and endpoints of overflights, all inside the basemap. East
/// Midlands is DHL's UK hub.
const EDGE_AIRPORTS: [&str; 23] = [
    "EGLL", "EGNX", "EIDW", "LFPG", "EHAM", "EBBR", "LSZH", "LSGG", "LOWW", "LKPR", "EPWA", "EKCH",
    "ESSA", "ENGM", "EFHK", "LHBP", "LROP", "LTFM", "LIRF", "LIMC", "LEBL", "LEMD", "LEPA",
];

/// Radius within which populated places count towards an airport's catchment.
const CATCHMENT_RADIUS_METERS: f64 = 100_000.0;

impl WorldSettings {
    /// Germany plus its edge airports; declination is about 3–5° east across
    /// the country in 2026.
    pub fn germany() -> Result<Self> {
        let mut edges = Vec::with_capacity(EDGE_AIRPORTS.len() + 1);
        for code in EDGE_AIRPORTS {
            edges.push(
                code.parse()
                    .with_context(|| format!("edge airport {code}"))?,
            );
        }
        Ok(Self {
            country: Some("DE".parse()?),
            edges,
            declination: Declination::Fixed(Degrees::new(4.0)?),
        })
    }

    /// Every aerodrome on the globe (plan M5 D96); no edges, the world is
    /// closed.
    pub fn globe() -> Result<Self> {
        Ok(Self {
            country: None,
            edges: Vec::new(),
            declination: Declination::Learnt,
        })
    }
}

/// Designator offsets from every runway end with a surveyed true heading.
fn learn_offsets(
    records: &[RunwayRecord],
    airports: &HashMap<String, (AirportDefinition, Fallbacks)>,
) -> DesignatorOffsets {
    let mut samples = Vec::with_capacity(records.len());
    for record in records {
        let Some((airport, _)) = airports.get(&record.airport_ident) else {
            continue;
        };
        for (ident, heading) in [
            (&record.le_ident, record.le_heading_degT),
            (&record.he_ident, record.he_heading_degT),
        ] {
            if let (Some(ident), Some(heading)) = (ident.as_deref(), heading)
                && let Some(offset) = offset_of(ident, heading)
            {
                samples.push((airport.location, offset));
            }
        }
    }
    DesignatorOffsets::new(samples)
}

/// Sum of the populations within [`CATCHMENT_RADIUS_METERS`] of `location`.
fn catchment(location: LatLon, places: &[PopulatedPlace]) -> u32 {
    places
        .iter()
        .filter(|place| place.location.distance_to(location).value() <= CATCHMENT_RADIUS_METERS)
        .fold(0_u32, |sum, place| sum.saturating_add(place.population))
}

/// Counts of approximations applied to imported records.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Fallbacks(BTreeMap<&'static str, usize>);

impl Fallbacks {
    pub(crate) fn count(&mut self, reason: &'static str) {
        *self.0.entry(reason).or_default() += 1;
    }

    fn merge(&mut self, other: Self) {
        for (reason, count) in other.0 {
            *self.0.entry(reason).or_default() += count;
        }
    }
}

/// What the import did, for logging and review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Airports written.
    pub airports: usize,
    /// Runways written with surveyed geometry.
    pub surveyed_runways: usize,
    /// Runways written with synthesised geometry.
    pub approximate_runways: usize,
    /// Records dropped, by reason.
    pub skipped: BTreeMap<&'static str, usize>,
    /// Approximations applied to written records, by kind.
    pub fallbacks: BTreeMap<&'static str, usize>,
}

/// The imported world and how it was produced.
#[derive(Debug, Clone)]
pub struct WorldImport {
    /// The world file contents, sorted for stable diffs.
    pub definition: WorldDefinition,
    /// Statistics.
    pub report: ImportReport,
}

/// Imports every aerodrome of `settings.country` that has at least one usable
/// runway.
pub fn import_world<R: Read>(
    sources: OurAirportsSources<R>,
    settings: &WorldSettings,
    places: &[PopulatedPlace],
) -> Result<WorldImport> {
    let mut fallbacks = Fallbacks::default();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();

    // OurAirports ident → converted airport (whose ident may be its ICAO code),
    // with the fallbacks its conversion needed; those only count if the airport
    // is written.
    let mut airports: HashMap<String, (AirportDefinition, Fallbacks)> = HashMap::new();
    let mut seen = HashSet::new();
    for record in read_csv::<AirportRecord>(sources.airports, "airports.csv")? {
        let mut airport_fallbacks = Fallbacks::default();
        if let Some(mut airport) = convert_airport(
            &record,
            settings.country,
            &settings.edges,
            &mut airport_fallbacks,
        )? {
            if airport.kind != AirportKind::Airfield {
                airport.catchment = Some(catchment(airport.location, places));
            }
            if seen.insert(airport.ident) {
                airports.insert(record.ident, (airport, airport_fallbacks));
            } else {
                *skipped.entry("duplicate ident").or_default() += 1;
            }
        }
    }

    let records = read_csv::<RunwayRecord>(sources.runways, "runways.csv")?;
    let offsets = match settings.declination {
        Declination::Fixed(_) => DesignatorOffsets::default(),
        Declination::Learnt => learn_offsets(&records, &airports),
    };
    let mut runways = Vec::new();
    for record in &records {
        let Some((airport, _)) = airports.get(&record.airport_ident) else {
            continue;
        };
        let declination = match settings.declination {
            Declination::Fixed(degrees) => degrees,
            Declination::Learnt => offsets.at(airport.location).unwrap_or_else(|| {
                fallbacks.count("no designator offset nearby");
                Degrees::default()
            }),
        };
        match convert_runway(record, airport, declination, &mut fallbacks)? {
            Ok(runway) => runways.push(runway),
            Err(reason) => *skipped.entry(reason).or_default() += 1,
        }
    }

    let with_runways: HashSet<AirportIdent> = runways.iter().map(|runway| runway.airport).collect();
    let mut kept = Vec::with_capacity(airports.len());
    for (airport, airport_fallbacks) in airports.into_values() {
        if with_runways.contains(&airport.ident) {
            fallbacks.merge(airport_fallbacks);
            kept.push(airport);
        } else {
            *skipped.entry("airport without runways").or_default() += 1;
        }
    }
    let mut airports = kept;
    airports.sort_by_key(|airport| airport.ident);
    runways.sort_by_key(|runway| (runway.airport, runway.ends[0].designator()));

    let surveyed_runways = runways
        .iter()
        .filter(|r| r.geometry == GeometrySource::Surveyed)
        .count();
    let report = ImportReport {
        airports: airports.len(),
        surveyed_runways,
        approximate_runways: runways.len() - surveyed_runways,
        skipped,
        fallbacks: fallbacks.0,
    };
    Ok(WorldImport {
        definition: WorldDefinition {
            format_version: FORMAT_VERSION,
            countries: Vec::new(),
            airports,
            runways,
        },
        report,
    })
}

/// The closed airports in `airports.csv` by their codes, with their
/// locations: flight lists still name airports closed since (plan M5 D109).
/// A closed airport's old ICAO code often survives only among its keywords
/// (Tegel: `TXL, EDDT, …`), so those that look like one count too.
pub fn read_closed_airports(reader: impl Read) -> Result<Vec<(String, LatLon)>> {
    let mut out = Vec::new();
    for record in read_csv::<AirportRecord>(reader, "airports.csv")? {
        if record.kind != "closed" {
            continue;
        }
        let Ok(at) = LatLon::new(record.latitude_deg, record.longitude_deg) else {
            continue;
        };
        if let Some(icao) = record
            .icao_code
            .filter(|c| !c.is_empty() && *c != record.ident)
        {
            out.push((icao, at));
        }
        let old = record
            .keywords
            .iter()
            .flat_map(|k| k.split(','))
            .map(str::trim);
        for code in old.filter(|c| c.len() == 4 && c.bytes().all(|b| b.is_ascii_uppercase())) {
            if code != record.ident {
                out.push((code.to_owned(), at));
            }
        }
        out.push((record.ident, at));
    }
    Ok(out)
}

fn read_csv<T: DeserializeOwned>(reader: impl Read, file: &str) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    for (line, row) in csv::Reader::from_reader(reader)
        .into_deserialize()
        .enumerate()
    {
        rows.push(row.with_context(|| format!("{file}: cannot parse record {}", line + 1))?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use aerocosm_core::{
        LatLon,
        world::{AirportKind, GeometrySource, Surface, World},
    };
    use anyhow::{Context, Result};

    use super::*;
    use crate::natural_earth::PopulatedPlace;

    #[test]
    fn test_closed_airports_with_their_old_codes() -> Result<()> {
        let locations = read_closed_airports(Cursor::new(
            "id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,\
             scheduled_service,icao_code,iata_code,keywords\n1,DE-0876,closed,Tegel,52.56,13.29,\
             122,DE,no,,,\"TXL, EDDT, Otto \
             Lilienthal\"\n2,K00A,closed,Old,40.0,-75.0,10,US,no,KXYZ,,\"KOLD\"\n3,KLAX,\
             large_airport,Open,33.9,-118.4,125,US,yes,KLAX,LAX,\n4,BAD,closed,Broken,95.0,0.0,0,\
             US,no,,,\n",
        ))?;
        let codes: Vec<&str> = locations.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, ["EDDT", "DE-0876", "KXYZ", "KOLD", "K00A"]);
        Ok(())
    }

    const AIRPORTS_HEADER: &str = "id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,\
                                   continent,iso_country,iso_region,municipality,\
                                   scheduled_service,icao_code,iata_code,gps_code,local_code,\
                                   home_link,wikipedia_link,keywords";
    const RUNWAYS_HEADER: &str = "id,airport_ref,airport_ident,length_ft,width_ft,surface,lighted,\
                                  closed,le_ident,le_latitude_deg,le_longitude_deg,\
                                  le_elevation_ft,le_heading_degT,le_displaced_threshold_ft,\
                                  he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,\
                                  he_heading_degT,he_displaced_threshold_ft";

    fn airports() -> String {
        [
            AIRPORTS_HEADER,
            r#"1,EDDF,large_airport,"Frankfurt am Main Airport",50.033333,8.570556,364,EU,DE,DE-HE,Frankfurt,yes,EDDF,FRA,EDDF,,,,"#,
            "2,EDFE,small_airport,Egelsbach Airport,49.959999,8.645833,384,EU,DE,DE-HE,Egelsbach,no,EDFE,QEF,EDFE,,,,",
            "3,DE-0004,small_airport,Local Field With Icao,51.0,10.0,,EU,DE,DE-TH,Somewhere,no,EDUG,,EDUG,,,,",
            "4,DE-0062,small_airport,Local Field,51.5,10.5,600,EU,DE,DE-TH,Elsewhere,no,,,,,,,",
            "5,EDXX,heliport,Some Heliport,50.0,9.0,300,EU,DE,DE-HE,,no,,,,,,,",
            "6,LFPG,large_airport,Charles de Gaulle,49.0,2.5,392,EU,FR,FR-IDF,Paris,yes,LFPG,CDG,LFPG,,,,",
            "7,EDNR,medium_airport,No Runways,48.0,11.0,,EU,DE,DE-BY,,yes,EDNR,,EDNR,,,,",
            "8,EDMX,closed,Closed Field,48.0,11.0,1500,EU,DE,DE-BY,,no,,,,,,,",
        ]
        .join("\n")
    }

    fn runways() -> String {
        [
            RUNWAYS_HEADER,
            // Surveyed, consistent.
            "1,1,EDDF,13123,197,CON,1,0,07C,50.032642,8.534615,,69.6,,25C,50.045049,8.586911,,249.\
             6,",
            // Surveyed coordinates far too short for the declared length: falls
            // back.
            "2,1,EDDF,13123,148,ASP,1,0,18,50.0,8.53,,180,,36,49.99,8.53,,0,",
            // No coordinates: synthesised from the designator.
            "3,2,EDFE,2756,66,ASP,1,0,09,,,,,,27,,,,,",
            // Helipad designators are skipped.
            "4,2,EDFE,100,100,ASP,0,0,H1,,,,,,H1,,,,,",
            // Closed runway.
            "5,2,EDFE,2000,60,GRS,0,1,03,,,,,,21,,,,,",
            "6,3,DE-0004,1800,,Gras,0,0,08,,,,,,26,,,,,",
            "7,4,DE-0062,1500,80,UNK,0,0,04,,,,45.0,,22,,,,225.0,",
            // Missing length.
            "8,4,DE-0062,,80,GRS,0,0,13,,,,,,31,,,,,",
            "9,6,LFPG,13829,148,CON,1,0,08L,49.0,2.5,,85,,26R,49.0,2.7,,265,",
        ]
        .join("\n")
    }

    fn import() -> Result<WorldImport> {
        let sources = OurAirportsSources {
            airports: Cursor::new(airports().into_bytes()),
            runways: Cursor::new(runways().into_bytes()),
        };
        import_world(sources, &WorldSettings::germany()?, &places()?)
    }

    fn import_globe() -> Result<WorldImport> {
        let sources = OurAirportsSources {
            airports: Cursor::new(airports().into_bytes()),
            runways: Cursor::new(runways().into_bytes()),
        };
        import_world(sources, &WorldSettings::globe()?, &places()?)
    }

    #[test]
    fn test_the_globe_takes_every_country_without_edges() -> Result<()> {
        let world = import_globe()?.definition;
        let paris = world
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "LFPG")
            .context("Paris")?;
        assert_eq!(paris.kind, AirportKind::Major);
        assert!(paris.scheduled);
        assert!(paris.catchment.is_some());
        assert!(world.airports.iter().all(|a| a.kind != AirportKind::Edge));
        let egelsbach = world
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "EDFE")
            .context("Egelsbach")?;
        assert!(!egelsbach.scheduled);
        // Germany alone takes Paris only as an edge.
        let germany = import()?.definition;
        let edge = germany
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "LFPG")
            .context("Paris as an edge")?;
        assert_eq!(edge.kind, AirportKind::Edge);
        Ok(())
    }

    #[test]
    fn test_the_globe_learns_how_designators_point() -> Result<()> {
        let heading = |world: &WorldDefinition| -> Result<f64> {
            let runway = world
                .runways
                .iter()
                .find(|r| r.airport.as_str() == "EDFE")
                .context("Egelsbach 09/27")?;
            Ok(runway.ends[0].true_heading().value())
        };
        // Eight surveyed ends nearby run -0.4° (EDDF 07C/25C), 0° (EDDF
        // 18/36) and +5° (LFPG, DE-0062) from their designators: +2.4° on
        // average.
        let learnt = heading(&import_globe()?.definition)?;
        assert!((learnt - 92.4).abs() < 0.05, "{learnt}");
        // Germany alone uses its fixed 4°.
        let fixed = heading(&import()?.definition)?;
        assert!((fixed - 94.0).abs() < 1e-9, "{fixed}");
        Ok(())
    }

    /// One place 20 km from Frankfurt, one 150 km away.
    fn places() -> Result<Vec<PopulatedPlace>> {
        Ok(vec![
            PopulatedPlace {
                location: LatLon::new(50.11, 8.68)?,
                population: 700_000,
            },
            PopulatedPlace {
                location: LatLon::new(51.34, 8.55)?,
                population: 1_000_000,
            },
        ])
    }

    #[test]
    fn test_import_world_selects_and_classifies_airports() -> Result<()> {
        let import = import()?;
        let idents: Vec<_> = import
            .definition
            .airports
            .iter()
            .map(|a| a.ident.to_string())
            .collect();
        assert_eq!(
            idents,
            ["DE-0062", "EDDF", "EDFE", "EDUG", "LFPG"],
            "sorted; heliports, closed, runway-less and non-edge foreign airports dropped"
        );
        let kind = |ident: &str| {
            import
                .definition
                .airports
                .iter()
                .find(|a| a.ident.as_str() == ident)
                .map(|a| a.kind)
        };
        assert_eq!(kind("EDDF"), Some(AirportKind::Major));
        assert_eq!(kind("EDFE"), Some(AirportKind::Airfield));
        assert_eq!(kind("DE-0062"), Some(AirportKind::Airfield));
        Ok(())
    }

    #[test]
    fn test_import_world_prefers_icao_code_as_ident() -> Result<()> {
        let import = import()?;
        let edug = import
            .definition
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "EDUG")
            .context("EDUG")?;
        assert_eq!(edug.icao.map(|c| c.to_string()).as_deref(), Some("EDUG"));
        assert!(
            (edug.elevation.value()).abs() < f64::EPSILON,
            "missing elevation defaults to 0"
        );
        let local = import
            .definition
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "DE-0062")
            .context("local")?;
        assert_eq!(local.icao, None);
        let runway = import
            .definition
            .runways
            .iter()
            .find(|r| r.airport.as_str() == "EDUG")
            .context("EDUG runway")?;
        assert_eq!(runway.surface, Surface::Grass);
        Ok(())
    }

    #[test]
    fn test_import_world_runway_geometry_sources() -> Result<()> {
        let import = import()?;
        let geometry = |airport: &str, designator: &str| {
            import
                .definition
                .runways
                .iter()
                .find(|r| {
                    r.airport.as_str() == airport
                        && r.ends[0].designator().to_string() == designator
                })
                .map(|r| r.geometry)
        };
        assert_eq!(geometry("EDDF", "07C"), Some(GeometrySource::Surveyed));
        assert_eq!(geometry("EDDF", "18"), Some(GeometrySource::Approximate));
        assert_eq!(geometry("EDFE", "09"), Some(GeometrySource::Approximate));
        assert_eq!(geometry("DE-0062", "04"), Some(GeometrySource::Approximate));
        assert_eq!(import.definition.runways.len(), 6);
        Ok(())
    }

    #[test]
    fn test_import_world_synthesis_uses_data_heading_or_designator() -> Result<()> {
        let import = import()?;
        let heading = |airport: &str| {
            import
                .definition
                .runways
                .iter()
                .find(|r| r.airport.as_str() == airport)
                .map(|r| r.ends[0].true_heading().value())
        };
        let local = heading("DE-0062").context("DE-0062")?;
        assert!((local - 45.0).abs() < 1e-9, "heading from data: {local}");
        let egelsbach = heading("EDFE").context("EDFE")?;
        assert!(
            (egelsbach - 94.0).abs() < 1e-9,
            "designator 09 + 4° declination: {egelsbach}"
        );
        Ok(())
    }

    #[test]
    fn test_import_world_reports_skips_and_fallbacks() -> Result<()> {
        let report = import()?.report;
        assert_eq!(report.airports, 5);
        assert_eq!(report.surveyed_runways, 1);
        assert_eq!(report.approximate_runways, 5);
        assert_eq!(report.skipped.get("closed"), Some(&1));
        assert_eq!(report.skipped.get("unparseable designator"), Some(&1));
        assert_eq!(report.skipped.get("missing length"), Some(&1));
        assert_eq!(report.skipped.get("airport without runways"), Some(&1));
        assert_eq!(
            report.fallbacks.get("surveyed geometry inconsistent"),
            Some(&2)
        );
        assert_eq!(report.fallbacks.get("missing elevation"), Some(&1));
        assert_eq!(report.fallbacks.get("default width"), Some(&1));
        Ok(())
    }

    #[test]
    fn test_import_world_marks_listed_foreign_airports_as_edge() -> Result<()> {
        let import = import()?;
        let paris = import
            .definition
            .airports
            .iter()
            .find(|a| a.ident.as_str() == "LFPG")
            .context("LFPG")?;
        assert_eq!(paris.kind, AirportKind::Edge);
        assert_eq!(paris.country.as_str(), "FR");
        Ok(())
    }

    #[test]
    fn test_import_world_computes_catchment_for_commercial_airports() -> Result<()> {
        let import = import()?;
        let catchment = |ident: &str| {
            import
                .definition
                .airports
                .iter()
                .find(|a| a.ident.as_str() == ident)
                .map(|a| a.catchment)
        };
        assert_eq!(
            catchment("EDDF"),
            Some(Some(700_000)),
            "only places within 100 km"
        );
        assert_eq!(
            catchment("LFPG"),
            Some(Some(0)),
            "edge airports get a catchment too"
        );
        assert_eq!(catchment("EDFE"), Some(None), "airfields have none");
        Ok(())
    }

    #[test]
    fn test_import_world_output_is_a_valid_world() -> Result<()> {
        let world = World::from_definition(import()?.definition)?;
        assert_eq!(world.airport_count(), 5);
        assert_eq!(world.runway_count(), 6);
        Ok(())
    }

    #[test]
    fn test_import_world_rejects_malformed_csv() -> Result<()> {
        let sources = OurAirportsSources {
            airports: Cursor::new(b"id,ident\n1,EDDF".to_vec()),
            runways: Cursor::new(runways().into_bytes()),
        };
        let error = import_world(sources, &WorldSettings::germany()?, &[])
            .err()
            .context("must fail")?;
        assert!(format!("{error:#}").contains("airports.csv"), "{error:#}");
        Ok(())
    }
}
