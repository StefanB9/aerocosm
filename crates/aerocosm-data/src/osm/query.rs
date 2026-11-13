//! The Overpass queries the ground import's downloads come from (plan M4
//! D64), so anyone can regenerate them.

use aerocosm_core::{
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use anyhow::{Context, Result};

/// Half the height and width of the query box around the reference point,
/// in degrees: about 5.5 × 5.7 km either way, enough for Germany's largest
/// airports.
const HALF_LAT: f64 = 0.05;
const HALF_LON: f64 = 0.08;

/// The airports with a ground layout: German airports with movement targets,
/// by ident.
pub fn ground_airports(world: &World, traffic: &TrafficTargets) -> Vec<AirportIdent> {
    let mut airports: Vec<AirportIdent> = traffic
        .airports()
        .map(aerocosm_core::traffic::AirportTarget::airport)
        .filter(|ident| {
            world
                .airport_by_ident(*ident)
                .and_then(|id| world.airport(id))
                .is_some_and(|airport| airport.country().as_str() == "DE")
        })
        .collect();
    airports.sort_unstable();
    airports
}

/// The Overpass QL query for `airport`'s ground features: a box around its
/// reference point; the import clips to the aerodrome outline.
pub fn overpass_query(world: &World, airport: AirportIdent) -> Result<String> {
    let location = world
        .airport_by_ident(airport)
        .and_then(|id| world.airport(id))
        .with_context(|| format!("{airport} is not in the world"))?
        .location();
    let (lat, lon) = (location.latitude().value(), location.longitude().value());
    Ok(format!(
        "[out:json][timeout:180][bbox:{:.3},{:.3},{:.3},{:.3}];(way[\"aeroway\"~\"\
         ^(aerodrome|taxiway|taxilane|runway|parking_position|apron|terminal|holding_position)$\"\
         ];node[\"aeroway\"~\"^(parking_position|holding_position|gate)$\"];);out body;>;out skel \
         qt;",
        lat - HALF_LAT,
        lon - HALF_LON,
        lat + HALF_LAT,
        lon + HALF_LON,
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn shipped() -> Result<(World, TrafficTargets)> {
        crate::shipped_model()
    }

    #[test]
    fn test_ground_airports_are_the_german_scheduled_airports() -> Result<()> {
        let (world, traffic) = shipped()?;
        let airports = ground_airports(&world, &traffic);
        let names: Vec<String> = airports.iter().map(ToString::to_string).collect();
        assert!(names.contains(&"EDDF".to_owned()) && names.contains(&"EDXW".to_owned()));
        assert!(!names.contains(&"EGLL".to_owned()));
        assert!(names.windows(2).all(|w| w[0] < w[1]));
        // Every German airport with scheduled service (until PR 3 goes
        // worldwide).
        assert!((30..50).contains(&names.len()), "{}", names.len());
        Ok(())
    }

    #[test]
    fn test_overpass_query_boxes_the_airport() -> Result<()> {
        let (world, _) = shipped()?;
        let query = overpass_query(&world, "EDDF".parse()?)?;
        assert!(
            query.starts_with("[out:json][timeout:180][bbox:49.977,8.478,50.077,8.638];"),
            "{query}"
        );
        assert!(query.contains("parking_position") && query.ends_with("out skel qt;"));
        assert!(!query.contains('\n'));
        assert!(overpass_query(&world, "ZZZZ".parse()?).is_err());
        Ok(())
    }
}
