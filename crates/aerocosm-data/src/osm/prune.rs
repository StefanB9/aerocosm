//! Stands the simulation could not use (plan M4 D66): those without a taxi
//! route to every departure runway in use with the prevailing wind, or from
//! an exit of every arrival runway, are left out. One-way taxiways mapped for
//! one flow strand some aprons.

use aerocosm_core::{
    ground::{AirportGround, AirportGroundDefinition},
    units::Meters,
    world::World,
};
use anyhow::{Context, Result};

use super::gate::Requirements;

/// Routes are checked for the smallest aircraft: stand sizes are limited
/// elsewhere.
const ANY_SPAN: f64 = 1.0;

/// Drops the stands of `definition` that cannot be routed to the departure
/// runways or from the arrival runways of `requirements`; returns how many.
pub(crate) fn prune_stands(
    definition: &mut AirportGroundDefinition,
    world: &World,
    requirements: &Requirements,
) -> Result<usize> {
    let airport = world
        .airport_by_ident(definition.airport)
        .with_context(|| format!("{} is not in the world", definition.airport))?;
    let layout = AirportGround::new(definition.clone(), world)
        .with_context(|| format!("invalid layout for {}", definition.airport))?;
    let span = Meters::new(ANY_SPAN)?;
    let ends: Vec<_> = world
        .runways_of(airport)
        .iter()
        .flat_map(|runway| runway.ends().iter())
        .collect();
    let entries: Vec<_> = requirements
        .departures
        .iter()
        .map(|designator| {
            ends.iter()
                .find(|end| end.designator() == *designator)
                .and_then(|end| layout.departure_entry(*designator, end.threshold()))
                .map(aerocosm_core::ground::Exit::node)
        })
        .collect();
    let usable: Vec<bool> = layout
        .stands()
        .iter()
        .map(|stand| {
            let out = entries.iter().all(|entry| {
                entry.is_some_and(|entry| layout.route(stand.node(), entry, span).is_some())
            });
            let back = requirements.arrivals.iter().all(|designator| {
                layout
                    .exits()
                    .iter()
                    .filter(|exit| exit.runway() == *designator)
                    .any(|exit| layout.route(exit.node(), stand.node(), span).is_some())
            });
            out && back
        })
        .collect();
    let before = definition.stands.len();
    let mut keep = usable.into_iter();
    definition.stands.retain(|_| keep.next().unwrap_or(false));
    Ok(before - definition.stands.len())
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{
        ground::{EdgeDefinition, EdgeKind, GroundDefinition, StandDefinition},
        units::Degrees,
    };
    use anyhow::{Context, Result};

    use super::*;
    use crate::osm::gate::requirements;

    #[test]
    fn test_prune_drops_stands_that_cannot_taxi_out() -> Result<()> {
        let (world, traffic) = crate::shipped_model()?;
        let mut definition =
            GroundDefinition::from_asset(include_bytes!("../../../../assets/ground/E.ground.bin"))?
                .airports
                .into_iter()
                .find(|a| a.airport.as_str() == "EDDF")
                .context("EDDF")?;
        let frankfurt = world.airport_by_ident(definition.airport).context("EDDF")?;
        let needs = requirements(&world, frankfurt, &traffic);
        // The shipped layout is already pruned.
        let shipped = definition.stands.len();
        assert_eq!(prune_stands(&mut definition, &world, &needs)?, 0);
        // A stand at the end of a one-way lane leading only into it.
        let from = definition.stands[0].node;
        let near = definition.nodes[from as usize];
        let node = u32::try_from(definition.nodes.len())?;
        definition.nodes.push([near[0] + 0.001, near[1]]);
        definition.edges.push(EdgeDefinition {
            from,
            to: node,
            kind: EdgeKind::Taxilane,
            name: None,
            oneway: true,
            max_wingspan: None,
            points: Vec::new(),
        });
        definition.stands.push(StandDefinition {
            name: "TRAP".parse()?,
            node,
            heading: Degrees::new(0.0)?,
            max_wingspan: Meters::new(36.0)?,
            area: None,
            cargo: false,
        });
        assert_eq!(prune_stands(&mut definition, &world, &needs)?, 1);
        assert_eq!(definition.stands.len(), shipped);
        assert!(definition.stands.iter().all(|s| s.name.as_str() != "TRAP"));
        Ok(())
    }
}
