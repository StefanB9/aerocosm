//! Converting `OurAirports` runway rows, synthesising geometry where needed
//! (D18).

use aerocosm_core::{
    Id, LatLon,
    units::{Degrees, Feet, Meters},
    world::{
        AirportDefinition, GeometrySource, Runway, RunwayDefinition, RunwayDesignator, RunwayEnd,
        Surface,
    },
};
use anyhow::Result;

use super::{Fallbacks, records::RunwayRecord, surface::normalize_surface};

/// Why a runway row was not imported.
pub(super) type SkipReason = &'static str;

/// Width assumed when the data has none: typical 30 m for paved strips, 25 m
/// otherwise.
fn default_width(surface: Surface) -> Result<Meters> {
    Meters::new(if surface == Surface::Paved {
        30.0
    } else {
        25.0
    })
}

/// Converts a row belonging to `airport`, or explains why it is skipped.
pub(super) fn convert_runway(
    record: &RunwayRecord,
    airport: &AirportDefinition,
    declination: Degrees,
    fallbacks: &mut Fallbacks,
) -> Result<Result<RunwayDefinition, SkipReason>> {
    if record.closed.as_deref() == Some("1") {
        return Ok(Err("closed"));
    }
    let designators = record
        .le_ident
        .as_deref()
        .zip(record.he_ident.as_deref())
        .and_then(|(le, he)| Some((le.parse::<RunwayDesignator>().ok()?, he.parse().ok()?)));
    let Some((low, high)) = designators else {
        return Ok(Err("unparseable designator"));
    };
    if low.reciprocal() != high {
        return Ok(Err("non-reciprocal designators"));
    }
    let Some(length) = record.length_ft.filter(|feet| *feet > 0.0) else {
        return Ok(Err("missing length"));
    };
    let length = Feet::new(length)?.to_meters();
    let surface = normalize_surface(record.surface.as_deref().unwrap_or_default());
    let width = if let Some(feet) = record.width_ft.filter(|feet| *feet > 0.0) {
        Feet::new(feet)?.to_meters()
    } else {
        fallbacks.count("default width");
        default_width(surface)?
    };
    let displaced = |feet: Option<f64>| -> Result<Meters> {
        Ok(Feet::new(feet.unwrap_or(0.0).max(0.0))?.to_meters())
    };
    let displaced = [
        displaced(record.le_displaced_threshold_ft)?,
        displaced(record.he_displaced_threshold_ft)?,
    ];
    let base = Base {
        designators: [low, high],
        length,
        width,
        surface,
        displaced,
        airport,
    };

    let thresholds = match (
        record.le_latitude_deg.zip(record.le_longitude_deg),
        record.he_latitude_deg.zip(record.he_longitude_deg),
    ) {
        (Some((lat1, lon1)), Some((lat2, lon2))) => LatLon::new(lat1, lon1)
            .ok()
            .zip(LatLon::new(lat2, lon2).ok()),
        _ => None,
    };
    if let Some((start, end)) = thresholds {
        let heading = |given: Option<f64>, from: LatLon, to: LatLon| match given {
            Some(degrees) => Degrees::new(degrees),
            None => Ok(from.initial_bearing_to(to)),
        };
        let surveyed = base.definition(
            [start, end],
            [
                heading(record.le_heading_degT, start, end)?,
                heading(record.he_heading_degT, end, start)?,
            ],
            GeometrySource::Surveyed,
        )?;
        if Runway::new(Id::new(0), &surveyed).is_ok() {
            return Ok(Ok(surveyed));
        }
        fallbacks.count("surveyed geometry inconsistent");
    }

    let centre = match thresholds {
        Some((start, end)) => start.interpolate(end, 0.5)?,
        None => airport.location,
    };
    let heading = match record.le_heading_degT {
        Some(degrees) => Degrees::new(degrees)?,
        // Designators are magnetic headings in tens of degrees.
        None => Degrees::new(f64::from(low.number()) * 10.0 + declination.value())?,
    };
    let synthesised = base.synthesise(centre, heading)?;
    if Runway::new(Id::new(0), &synthesised).is_err() {
        return Ok(Err("invalid geometry"));
    }
    Ok(Ok(synthesised))
}

/// Everything about a runway except where its ends are.
struct Base<'a> {
    designators: [RunwayDesignator; 2],
    length: Meters,
    width: Meters,
    surface: Surface,
    displaced: [Meters; 2],
    airport: &'a AirportDefinition,
}

impl Base<'_> {
    fn definition(
        &self,
        thresholds: [LatLon; 2],
        headings: [Degrees; 2],
        geometry: GeometrySource,
    ) -> Result<RunwayDefinition> {
        Ok(RunwayDefinition {
            airport: self.airport.ident,
            ends: [
                RunwayEnd::new(
                    self.designators[0],
                    thresholds[0],
                    headings[0],
                    self.displaced[0],
                )?,
                RunwayEnd::new(
                    self.designators[1],
                    thresholds[1],
                    headings[1],
                    self.displaced[1],
                )?,
            ],
            length: self.length,
            width: self.width,
            surface: self.surface,
            geometry,
        })
    }

    /// A runway of the declared length centred on `centre` along true
    /// `heading`.
    fn synthesise(&self, centre: LatLon, heading: Degrees) -> Result<RunwayDefinition> {
        let heading = heading.normalized();
        let reciprocal = Degrees::new(heading.value() + 180.0)?.normalized();
        let half = self.length * 0.5;
        let start = centre.destination(reciprocal, half);
        let end = centre.destination(heading, half);
        self.definition(
            [start, end],
            [heading, reciprocal],
            GeometrySource::Approximate,
        )
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::world::AirportKind;
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    fn airfield(lat: f64, lon: f64) -> Result<AirportDefinition> {
        Ok(AirportDefinition {
            ident: "DE-0001".parse()?,
            icao: None,
            iata: None,
            name: "Field".to_owned(),
            location: LatLon::new(lat, lon)?,
            elevation: Feet::new(0.0)?,
            kind: AirportKind::Airfield,
            country: "DE".parse()?,
        })
    }

    #[test]
    fn test_default_width_by_surface() -> Result<()> {
        assert!((default_width(Surface::Paved)?.value() - 30.0).abs() < 1e-9);
        assert!((default_width(Surface::Grass)?.value() - 25.0).abs() < 1e-9);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_synthesised_runways_pass_core_validation(
            number in 1_u8..=18,
            length in 100.0_f64..5000.0,
            heading in 0.0_f64..360.0,
            lat in 47.0_f64..55.0,
            lon in 5.0_f64..15.0,
        ) {
            let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
            let low: RunwayDesignator = format!("{number:02}").parse().map_err(fail)?;
            let airport = airfield(lat, lon).map_err(fail)?;
            let base = Base {
                designators: [low, low.reciprocal()],
                length: Meters::new(length).map_err(fail)?,
                width: Meters::new(30.0).map_err(fail)?,
                surface: Surface::Grass,
                displaced: [Meters::default(); 2],
                airport: &airport,
            };
            let definition = base
                .synthesise(airport.location, Degrees::new(heading).map_err(fail)?)
                .map_err(fail)?;
            prop_assert!(Runway::new(Id::new(0), &definition).is_ok());
            prop_assert_eq!(definition.geometry, GeometrySource::Approximate);
        }
    }
}
