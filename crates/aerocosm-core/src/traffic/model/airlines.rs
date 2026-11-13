//! Airlines by rule (plan M5 D110): who flies passengers or cargo, with
//! which fleet, and which of their airports are hubs.

use crate::{
    AircraftCatalog,
    airline::{AirlineKind, Colour, FleetShare},
    traffic::observed::ObservedAirline,
};

/// Words in an airline's name that mark it as a cargo carrier.
const CARGO_WORDS: [&str; 4] = ["cargo", "freight", "parcel", "logistic"];
/// An airline is cargo when at least this share of its flights are on
/// freighter-only types (`FedEx` 0.52, DHL's European Air Transport 0.43;
/// Ethiopian, with its 777 freighters among airliners, 0.35).
const FREIGHTER_SHARE: f64 = 0.4;
/// A hub holds at least this share of the airline's departures…
const HUB_SHARE_OF_AIRLINE: f64 = 0.08;
/// …and the airline flies at least this share of the airport's.
const HUB_SHARE_OF_AIRPORT: f64 = 0.25;

/// What the rules make of an observed callsign prefix.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Verdict {
    /// An airline of this kind with this fleet.
    Airline(AirlineKind, Vec<FleetShare>),
    /// Mostly types outside the airliner catalog (business jets, light
    /// aircraft), or no types seen.
    Dropped,
}

/// Classifies `airline` by its observed types and name.
pub(super) fn classify(airline: &ObservedAirline, aircraft: &AircraftCatalog) -> Verdict {
    let total: u64 = airline.types.iter().map(|(_, n)| u64::from(*n)).sum();
    let mut fleet: Vec<(crate::world::codes::AircraftTypeCode, u64)> = Vec::new();
    let mut freighters = 0_u64;
    for (code, flights) in &airline.types {
        let Some(aircraft) = aircraft.resolve(*code).and_then(|id| aircraft.get(id)) else {
            continue;
        };
        if aircraft.is_freighter() {
            freighters += u64::from(*flights);
        }
        match fleet.iter_mut().find(|(c, _)| *c == aircraft.code()) {
            Some((_, n)) => *n += u64::from(*flights),
            None => fleet.push((aircraft.code(), u64::from(*flights))),
        }
    }
    let resolved: u64 = fleet.iter().map(|(_, n)| n).sum();
    if total == 0 || resolved * 2 < total {
        return Verdict::Dropped;
    }
    let name = airline.name.to_lowercase();
    // Counts of flights, far below 2^53.
    #[allow(clippy::cast_precision_loss)]
    let cargo = freighters as f64 >= resolved as f64 * FREIGHTER_SHARE
        || CARGO_WORDS.iter().any(|word| name.contains(word));
    if !cargo {
        // A passenger airline's freighters fly cargo the passenger layer does
        // not carry; and a freighter-only code may also name a passenger
        // variant (the 777-200LR is a B77L too).
        fleet.retain(|(code, _)| {
            aircraft
                .by_code(*code)
                .and_then(|id| aircraft.get(id))
                .is_some_and(|a| !a.is_freighter())
        });
        if fleet.is_empty() {
            return Verdict::Dropped;
        }
    }
    let resolved: u64 = fleet.iter().map(|(_, n)| n).sum();
    fleet.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    #[allow(clippy::cast_precision_loss)]
    let shares = fleet
        .into_iter()
        .map(|(aircraft, flights)| FleetShare {
            aircraft,
            share: flights as f64 / resolved as f64,
        })
        .collect();
    let kind = if cargo {
        AirlineKind::Cargo
    } else {
        AirlineKind::Passenger
    };
    Verdict::Airline(kind, shares)
}

/// Whether an airport where the airline flies `own` departures, of its
/// `all` in total and of `airport` at the airport, is one of its hubs.
pub(super) fn is_hub(own: f64, all: f64, airport: f64) -> bool {
    all > 0.0
        && airport > 0.0
        && own / all >= HUB_SHARE_OF_AIRLINE
        && own / airport >= HUB_SHARE_OF_AIRPORT
}

/// A colour for the airline with designator `code`: a hue from its letters,
/// saturated and mid-dark so that it reads on the map.
pub(super) fn colour(code: &str) -> Colour {
    // FNV-1a over the letters spreads neighbouring codes round the wheel.
    let hash = code.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let hue = f64::from(hash % 360);
    let (saturation, lightness) = (0.65, 0.42);
    let chroma = (1.0 - (2.0 * lightness - 1.0_f64).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let channels = match hash % 360 / 60 {
        0 => [chroma, second, 0.0],
        1 => [second, chroma, 0.0],
        2 => [0.0, chroma, second],
        3 => [0.0, second, chroma],
        4 => [second, 0.0, chroma],
        _ => [chroma, 0.0, second],
    };
    let base = lightness - chroma / 2.0;
    // SAFETY: each channel lies in [0, 1] before scaling, so the cast fits.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |value: f64| ((value + base).clamp(0.0, 1.0) * 255.0).round() as u8;
    Colour::from_rgb(channels.map(byte))
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use proptest::prelude::*;

    use super::*;

    fn catalog() -> Result<AircraftCatalog> {
        AircraftCatalog::from_json(include_str!("../../../../../assets/aircraft/types.json"))
    }

    fn observed(name: &str, types: &[(&str, u32)]) -> Result<ObservedAirline> {
        Ok(ObservedAirline {
            icao: "TST".parse()?,
            name: name.to_owned(),
            callsign: None,
            departures: Vec::new(),
            types: types
                .iter()
                .map(|(code, n)| Ok((code.parse()?, *n)))
                .collect::<Result<_>>()?,
        })
    }

    #[test]
    fn test_classify_by_types_and_name() -> Result<()> {
        let aircraft = catalog()?;
        // A passenger airline; the 737-600 flies as a 737-700, the 777F
        // leaves its fleet.
        let Verdict::Airline(kind, fleet) = classify(
            &observed(
                "Sample Air",
                &[("A320", 60), ("B736", 30), ("C172", 10), ("B77L", 5)],
            )?,
            &aircraft,
        ) else {
            bail!("an airline");
        };
        assert_eq!(kind, AirlineKind::Passenger);
        let codes: Vec<String> = fleet.iter().map(|f| f.aircraft.to_string()).collect();
        assert_eq!(codes, ["A320", "B737"]);
        assert!((fleet.iter().map(|f| f.share).sum::<f64>() - 1.0).abs() < 1e-12);
        // Freighter-only types make a cargo airline…
        assert!(matches!(
            classify(
                &observed("Sample", &[("MD11", 40), ("B763", 60)])?,
                &aircraft
            ),
            Verdict::Airline(AirlineKind::Cargo, _)
        ));
        // …and so does the name, whatever it flies.
        assert!(matches!(
            classify(&observed("Sample Cargo", &[("B763", 60)])?, &aircraft),
            Verdict::Airline(AirlineKind::Cargo, _)
        ));
        // A 767 alone is a passenger airline.
        assert!(matches!(
            classify(&observed("Sample Express", &[("B763", 60)])?, &aircraft),
            Verdict::Airline(AirlineKind::Passenger, _)
        ));
        // Business jets and no types at all are dropped.
        assert_eq!(
            classify(&observed("Jets", &[("C68A", 60), ("A320", 10)])?, &aircraft),
            Verdict::Dropped
        );
        assert_eq!(
            classify(&observed("Unknown", &[])?, &aircraft),
            Verdict::Dropped
        );
        Ok(())
    }

    #[test]
    fn test_hubs_need_both_shares() {
        assert!(is_hub(300.0, 1000.0, 1000.0));
        assert!(!is_hub(50.0, 1000.0, 100.0), "small share of the airline");
        assert!(!is_hub(300.0, 1000.0, 3000.0), "small share of the airport");
        assert!(!is_hub(0.0, 0.0, 0.0));
    }

    #[test]
    fn test_colours_differ_and_repeat() {
        assert_eq!(colour("DLH"), colour("DLH"));
        assert_ne!(colour("DLH"), colour("DLA"));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Generated colours are neither washed out nor black.
        #[test]
        fn test_colours_are_mid_dark(code in "[A-Z]{3}") {
            let [r, g, b] = colour(&code).rgb();
            let (high, low) = (r.max(g).max(b), r.min(g).min(b));
            prop_assert!(high >= 150 && low <= 60, "{code}: {r} {g} {b}");
        }
    }
}
