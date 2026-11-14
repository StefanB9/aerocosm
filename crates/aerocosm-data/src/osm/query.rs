//! The Overpass queries the ground import's downloads come from (plan M4
//! D64), so anyone can regenerate them.

use aerocosm_core::{
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use anyhow::{Context, Result};

/// Half the height and width of the smallest query box around the
/// reference point, in degrees: about 5.5 × 5.7 km either way in Germany.
const HALF_LAT: f64 = 0.05;
const HALF_LON: f64 = 0.08;
/// The box reaches this far beyond the outermost runway end, degrees of
/// latitude (about 3.3 km; aprons and cargo areas lie beside the runways).
const MARGIN: f64 = 0.03;

/// The airports with a ground layout: every scheduled airport (plan M5
/// D103), by ident.
pub fn ground_airports(_world: &World, traffic: &TrafficTargets) -> Vec<AirportIdent> {
    let mut airports: Vec<AirportIdent> = traffic
        .airports()
        .map(aerocosm_core::traffic::AirportTarget::airport)
        .collect();
    airports.sort_unstable();
    airports
}

/// The `aeroway` values of the ways the import reads.
pub(crate) const WAY_AEROWAYS: [&str; 8] = [
    "aerodrome",
    "taxiway",
    "taxilane",
    "runway",
    "parking_position",
    "apron",
    "terminal",
    "holding_position",
];
/// The `aeroway` values of the tagged nodes the import reads.
pub(crate) const NODE_AEROWAYS: [&str; 3] = ["parking_position", "holding_position", "gate"];

/// The box round `airport` the import reads (south, west, north, east): the
/// smallest box round its reference point, grown to every runway end plus a
/// margin.
pub fn query_box(world: &World, airport: AirportIdent) -> Result<[f64; 4]> {
    let id = world
        .airport_by_ident(airport)
        .with_context(|| format!("{airport} is not in the world"))?;
    let location = world.airport(id).context("airport id")?.location();
    let (lat, lon) = (location.latitude().value(), location.longitude().value());
    let mut bbox = [
        lat - HALF_LAT,
        lon - HALF_LON,
        lat + HALF_LAT,
        lon + HALF_LON,
    ];
    let margin_lon = MARGIN / lat.to_radians().cos().max(0.1);
    for runway in world.runways_of(id) {
        for end in runway.ends() {
            let at = end.threshold();
            let (y, x) = (at.latitude().value(), at.longitude().value());
            bbox[0] = bbox[0].min(y - MARGIN);
            bbox[1] = bbox[1].min(x - margin_lon);
            bbox[2] = bbox[2].max(y + MARGIN);
            bbox[3] = bbox[3].max(x + margin_lon);
        }
    }
    Ok(bbox)
}

/// The Overpass QL query for `airport`'s ground features: its
/// [`query_box`]; the import clips to the aerodrome outline.
pub fn overpass_query(world: &World, airport: AirportIdent) -> Result<String> {
    let [south, west, north, east] = query_box(world, airport)?;
    Ok(format!(
        "[out:json][timeout:180][bbox:{south:.3},{west:.3},{north:.3},{east:.3}];(way[\"aeroway\"\
         ~\"^({})$\"];node[\"aeroway\"~\"^({})$\"];);out body;>;out skel qt;",
        WAY_AEROWAYS.join("|"),
        NODE_AEROWAYS.join("|"),
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    fn shipped() -> Result<(World, TrafficTargets)> {
        crate::shipped_model()
    }

    #[test]
    fn test_ground_airports_are_the_scheduled_airports() -> Result<()> {
        let (world, traffic) = shipped()?;
        let airports = ground_airports(&world, &traffic);
        let names: Vec<String> = airports.iter().map(ToString::to_string).collect();
        for ident in ["EDDF", "EDXW", "EGLL", "KLAX", "FAOR", "YSSY", "SBGR"] {
            assert!(names.contains(&ident.to_owned()), "{ident}");
        }
        assert!(names.windows(2).all(|w| w[0] < w[1]));
        // Every scheduled airport (plan M5 D103).
        assert_eq!(names.len(), traffic.airports().count());
        Ok(())
    }

    /// The query's bounding box: south, west, north, east.
    fn bbox(query: &str) -> Result<[f64; 4]> {
        let inner = query
            .split("[bbox:")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .context("a bounding box")?;
        let values: Vec<f64> = inner
            .split(',')
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()?;
        values.try_into().ok().context("four values")
    }

    #[test]
    fn test_overpass_query_boxes_the_airport() -> Result<()> {
        let (world, _) = shipped()?;
        let query = overpass_query(&world, "EDDF".parse()?)?;
        assert!(
            query.starts_with("[out:json][timeout:180][bbox:"),
            "{query}"
        );
        // At least the 5.5 × 5.7 km box round the reference point.
        let [south, west, north, east] = bbox(&query)?;
        assert!(south <= 49.977 && west <= 8.478 && north >= 50.077 && east >= 8.638);
        assert!(query.contains("parking_position") && query.ends_with("out skel qt;"));
        assert!(!query.contains('\n'));
        assert!(overpass_query(&world, "ZZZZ".parse()?).is_err());
        Ok(())
    }

    #[test]
    fn test_overpass_query_covers_every_runway_with_a_margin() -> Result<()> {
        let (world, _) = shipped()?;
        // Denver's runways spread over 13 km, beyond the default box.
        for ident in ["KDEN", "KDFW", "EDDF", "OMDB"] {
            let id = world.airport_by_ident(ident.parse()?).context("airport")?;
            let [south, west, north, east] = bbox(&overpass_query(&world, ident.parse()?)?)?;
            for runway in world.runways_of(id) {
                for end in runway.ends() {
                    let at = end.threshold();
                    let (lat, lon) = (at.latitude().value(), at.longitude().value());
                    assert!(
                        lat - south > 0.02 && north - lat > 0.02,
                        "{ident} {lat} in {south}..{north}"
                    );
                    let margin = 0.02 / lat.to_radians().cos();
                    assert!(
                        lon - west > margin && east - lon > margin,
                        "{ident} {lon} in {west}..{east}"
                    );
                }
            }
        }
        Ok(())
    }
}
