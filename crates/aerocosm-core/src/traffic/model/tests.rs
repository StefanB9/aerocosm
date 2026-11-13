use anyhow::{Context, Result};

use super::{calibrate::WORLD_DEPARTURES_PER_DAY, *};
use crate::airline::Network;

/// ADV-Monatsstatistik 12/2019, January–December 2019: scheduled and charter
/// movements ("Linien-, Pauschalflug-, Tramp- und Anforderungsverkehr"),
/// arrivals plus departures. Berlin is Tegel and Schönefeld together, the
/// traffic of today's BER. Validation only (plan M5 D105, D114).
const ADV_2019: [(&str, f64); 11] = [
    ("EDDF", 502_634.0),
    ("EDDM", 395_951.0),
    ("EDDB", 269_127.0),
    ("EDDL", 214_595.0),
    ("EDDH", 136_959.0),
    ("EDDK", 128_855.0),
    ("EDDS", 117_011.0),
    ("EDDP", 70_044.0),
    ("EDDV", 55_178.0),
    ("EDDN", 39_460.0),
    ("EDDW", 21_640.0),
];
/// All German airports in the same statistic.
const ADV_2019_GERMANY: f64 = 2_050_350.0;

fn shipped() -> Result<(World, WorldModel)> {
    let world = World::from_asset(include_bytes!("../../../../../assets/world/world.bin"))?;
    let observed =
        ObservedTraffic::from_asset(include_bytes!("../../../../../assets/traffic/observed.bin"))?;
    let aircraft =
        AircraftCatalog::from_json(include_str!("../../../../../assets/aircraft/types.json"))?;
    let model = derive(&world, &observed, &aircraft)?;
    Ok((world, model))
}

#[test]
fn test_bank_times_spread_over_the_day() {
    let minutes = |daily| -> Vec<u16> {
        bank_times(daily)
            .iter()
            .map(|t| t.minute_of_day())
            .collect()
    };
    assert_eq!(minutes(30.0), [520, 840, 1160], "three at least");
    assert_eq!(minutes(1000.0).len(), 8, "eight at most");
    assert_eq!(minutes(300.0).len(), 5);
    assert!(minutes(300.0).windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn test_the_world_model_covers_every_scheduled_airport() -> Result<()> {
    let (world, model) = shipped()?;
    let targets = model.targets();
    let scheduled = world
        .airports()
        .filter(|(_, a)| a.scheduled() && a.zone().is_some())
        .count();
    assert_eq!(targets.airports().count(), scheduled);
    let departures: f64 = targets
        .airports()
        .map(|t| f64::from(t.movements_per_year()) / 2.0 / DAYS)
        .sum();
    assert!(
        (departures / WORLD_DEPARTURES_PER_DAY - 1.0).abs() < 0.01,
        "{departures}"
    );
    // Every airport is a base of some airline.
    let mut based: std::collections::HashSet<_> = std::collections::HashSet::new();
    for airline in model.airlines().iter() {
        based.extend(airline.bases().iter().map(|b| b.airport));
    }
    // Except where no type of the catalog fits the runway (Inis Mór's 158 m
    // in OurAirports).
    let aircraft =
        AircraftCatalog::from_json(include_str!("../../../../../assets/aircraft/types.json"))?;
    let shortest = aircraft
        .iter()
        .map(|a| a.min_runway_length().value() * crate::schedule::RUNWAY_MARGIN)
        .fold(f64::INFINITY, f64::min);
    let bare: Vec<String> = targets
        .airports()
        .filter(|t| !based.contains(&t.airport()))
        .filter(|t| {
            world.airport_by_ident(t.airport()).is_some_and(|id| {
                world
                    .runways_of(id)
                    .iter()
                    .any(|r| r.length().value() >= shortest)
            })
        })
        .map(|t| t.airport().to_string())
        .collect();
    assert!(bare.is_empty(), "{bare:?}");
    let report = model.report();
    assert!(report.observed_airports > 400, "{report:?}");
    assert!(
        report.airlines > 300 && report.cargo_airlines >= 15,
        "{report:?}"
    );
    assert!(report.dropped_airlines > 100, "{report:?}");
    Ok(())
}

#[test]
fn test_airlines_follow_the_rules() -> Result<()> {
    let (_, model) = shipped()?;
    let airlines = model.airlines();
    let airline = |code: &str| -> Result<&crate::airline::Airline> {
        airlines
            .by_icao(code.parse()?)
            .and_then(|id| airlines.get(id))
            .with_context(|| code.to_string())
    };
    let lufthansa = airline("DLH")?;
    assert_eq!(lufthansa.kind(), AirlineKind::Passenger);
    assert_eq!(lufthansa.network(), Network::Hub);
    assert_eq!(airline("RYR")?.network(), Network::PointToPoint);
    for cargo in ["FDX", "UPS", "BCS", "GEC", "CLX"] {
        assert_eq!(airline(cargo)?.kind(), AirlineKind::Cargo, "{cargo}");
    }
    for passenger in ["ICE", "CFG", "IBS", "HKE", "ETH"] {
        assert_eq!(
            airline(passenger)?.kind(),
            AirlineKind::Passenger,
            "{passenger}"
        );
    }
    assert!(airlines.by_icao("EJA".parse()?).is_none(), "business jets");
    let frankfurt = model
        .targets()
        .airport("EDDF".parse()?)
        .context("Frankfurt")?;
    assert!(frankfurt.banks().len() >= 6, "{:?}", frankfurt.banks());
    assert_eq!(frankfurt.capacity().map(|c| c.per_hour), Some(100));
    assert_eq!(model.targets().runway_modes("EDDF".parse()?).len(), 2);
    let leipzig = model
        .targets()
        .airport("EDDP".parse()?)
        .context("Leipzig")?;
    assert!(leipzig.cargo_share() > 0.5, "{}", leipzig.cargo_share());
    Ok(())
}

#[test]
fn test_german_targets_match_adv_2019() -> Result<()> {
    let (world, model) = shipped()?;
    let mut failures = Vec::new();
    for (ident, movements) in ADV_2019 {
        let target = model
            .targets()
            .airport(ident.parse()?)
            .with_context(|| ident.to_owned())?;
        let ratio = f64::from(target.movements_per_year()) / movements;
        if !(0.75..=1.25).contains(&ratio) {
            failures.push(format!("{ident} {ratio:.2}"));
        }
    }
    let germany: f64 = model
        .targets()
        .airports()
        .filter(|t| {
            world
                .airport_by_ident(t.airport())
                .and_then(|id| world.airport(id))
                .is_some_and(|a| a.country().as_str() == "DE")
        })
        .map(|t| f64::from(t.movements_per_year()))
        .sum();
    let ratio = germany / ADV_2019_GERMANY;
    if !(0.9..=1.1).contains(&ratio) {
        failures.push(format!("Germany {ratio:.2}"));
    }
    assert!(failures.is_empty(), "{failures:?}");
    Ok(())
}
