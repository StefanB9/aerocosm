//! The quality gate (plan M4 D66): an airport's layout is used only if it
//! has enough stands and its network links stands to every runway end in use
//! with the prevailing wind (one-way taxiways are often mapped for one flow).

use std::collections::VecDeque;

use aerocosm_core::{
    Id,
    ground::AirportGroundDefinition,
    ops::active_runways_now,
    traffic::TrafficTargets,
    world::{Airport, RunwayDesignator, World},
};

use super::local::angle_between;

/// One stand per this many daily movements at least.
const MOVEMENTS_PER_STAND: f64 = 10.0;
/// Without curated modes, runways at least this fraction of the longest one
/// are in use.
const LONG_RUNWAY: f64 = 0.75;
/// Days per year, for daily movements.
const DAYS: f64 = 365.0;

/// What a layout must provide.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Requirements {
    pub(crate) min_stands: usize,
    pub(crate) arrivals: Vec<RunwayDesignator>,
    pub(crate) departures: Vec<RunwayDesignator>,
}

/// The requirements at `airport`: stands for its movement target and the
/// runway ends in use with the prevailing wind: those of its curated mode, or
/// of its long runways facing that way.
pub(crate) fn requirements(
    world: &World,
    airport: Id<Airport>,
    traffic: &TrafficTargets,
) -> Requirements {
    let ident = world
        .airport(airport)
        .map(aerocosm_core::world::Airport::ident);
    let target = ident.and_then(|ident| traffic.airport(ident));
    let daily = target.map_or(0.0, |t| f64::from(t.movements_per_year()) / DAYS);
    // SAFETY: a small non-negative count, rounded up.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_stands = ((daily / MOVEMENTS_PER_STAND).ceil() as usize).max(1);
    let modes = ident.map_or(&[][..], |ident| traffic.runway_modes(ident));
    let runways = world.runways_of(airport);
    let active = active_runways_now(runways, modes);
    let mut arrivals = Vec::new();
    let mut departures = Vec::new();
    if modes.is_empty() {
        // Mixed mode: every long runway's end facing the same way as the
        // best-aligned one may be used, depending on the aircraft.
        let flow = active
            .first()
            .map(|a| runways[a.runway].ends()[a.end].true_heading());
        let longest = runways
            .iter()
            .map(|r| r.length().value())
            .fold(0.0, f64::max);
        for runway in runways {
            if runway.length().value() < longest * LONG_RUNWAY {
                continue;
            }
            for end in runway.ends() {
                if flow.is_some_and(|flow| {
                    angle_between(end.true_heading().value(), flow.value()) <= 90.0
                }) {
                    arrivals.push(end.designator());
                    departures.push(end.designator());
                }
            }
        }
    } else {
        for use_ in &active {
            let designator = runways[use_.runway].ends()[use_.end].designator();
            if use_.arrivals {
                arrivals.push(designator);
            }
            if use_.departures {
                departures.push(designator);
            }
        }
    }
    for list in [&mut arrivals, &mut departures] {
        list.sort_unstable();
        list.dedup();
    }
    Requirements {
        min_stands,
        arrivals,
        departures,
    }
}

/// Nodes reachable from `sources` along edges, respecting one-way edges.
fn reachable(layout: &AirportGroundDefinition, sources: impl Iterator<Item = u32>) -> Vec<bool> {
    let n = layout.nodes.len();
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in &layout.edges {
        let (from, to) = (edge.from as usize, edge.to as usize);
        if from < n && to < n {
            next[from].push(to);
            if !edge.oneway {
                next[to].push(from);
            }
        }
    }
    let mut seen = vec![false; n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for source in sources {
        let source = source as usize;
        if source < n && !seen[source] {
            seen[source] = true;
            queue.push_back(source);
        }
    }
    while let Some(node) = queue.pop_front() {
        for &other in &next[node] {
            if !seen[other] {
                seen[other] = true;
                queue.push_back(other);
            }
        }
    }
    seen
}

/// `Ok` if `layout` meets `requirements`, else why not.
pub(crate) fn check(
    layout: &AirportGroundDefinition,
    requirements: &Requirements,
) -> Result<(), String> {
    if layout.stands.len() < requirements.min_stands {
        return Err(format!(
            "{} stands, {} needed",
            layout.stands.len(),
            requirements.min_stands
        ));
    }
    let from_stands = reachable(layout, layout.stands.iter().map(|s| s.node));
    for runway in &requirements.departures {
        let ok = layout
            .holding_positions
            .iter()
            .any(|h| h.runway == *runway && from_stands[h.node as usize]);
        if !ok {
            return Err(format!(
                "no holding position for {runway} reachable from a stand"
            ));
        }
    }
    for runway in &requirements.arrivals {
        let exits = layout.exits.iter().filter(|e| e.runway == *runway);
        let after = reachable(layout, exits.map(|e| e.node));
        if !layout.stands.iter().any(|s| after[s.node as usize]) {
            return Err(format!("no exit from {runway} leads to a stand"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{
        ground::{
            EdgeDefinition, EdgeKind, ExitDefinition, HoldingPositionDefinition, StandDefinition,
        },
        units::{Degrees, Meters},
    };
    use anyhow::{Context, Result};

    use super::*;

    fn edge(from: u32, to: u32, oneway: bool) -> EdgeDefinition {
        EdgeDefinition {
            from,
            to,
            kind: EdgeKind::Taxiway,
            name: None,
            oneway,
            max_wingspan: None,
            points: Vec::new(),
        }
    }

    /// Stand 0 – 1 – 2 (holding for 25) – 3 (exit from 07).
    fn layout() -> Result<AirportGroundDefinition> {
        Ok(AirportGroundDefinition {
            airport: "EDDF".parse()?,
            nodes: vec![[50.0, 8.5]; 4],
            edges: vec![edge(0, 1, false), edge(1, 2, false), edge(2, 3, false)],
            stands: vec![StandDefinition {
                name: "A1".parse()?,
                node: 0,
                heading: Degrees::new(0.0)?,
                max_wingspan: Meters::new(36.0)?,
                area: None,
            }],
            holding_positions: vec![HoldingPositionDefinition {
                node: 2,
                runway: "25".parse()?,
            }],
            exits: vec![ExitDefinition {
                runway: "07".parse()?,
                node: 3,
                distance: Meters::new(1_500.0)?,
                angle: Degrees::new(45.0)?,
            }],
            aprons: Vec::new(),
        })
    }

    fn requires(stands: usize) -> Result<Requirements> {
        Ok(Requirements {
            min_stands: stands,
            arrivals: vec!["07".parse()?],
            departures: vec!["25".parse()?],
        })
    }

    #[test]
    fn test_gate_passes_a_connected_layout() -> Result<()> {
        assert_eq!(check(&layout()?, &requires(1)?), Ok(()));
        Ok(())
    }

    #[test]
    fn test_gate_names_what_is_missing() -> Result<()> {
        assert_eq!(
            check(&layout()?, &requires(2)?),
            Err("1 stands, 2 needed".to_owned())
        );
        let mut one_way = layout()?;
        one_way.edges[0] = edge(1, 0, true);
        assert_eq!(
            check(&one_way, &requires(1)?),
            Err("no holding position for 25 reachable from a stand".to_owned())
        );
        let mut cut = layout()?;
        cut.edges.pop();
        assert_eq!(
            check(&cut, &requires(1)?),
            Err("no exit from 07 leads to a stand".to_owned())
        );
        let mut wrong_end = requires(1)?;
        wrong_end.departures = vec!["07".parse()?];
        assert!(check(&layout()?, &wrong_end).is_err());
        Ok(())
    }

    #[test]
    fn test_requirements_follow_modes_and_movements() -> Result<()> {
        let data = aerocosm_core_test_data()?;
        let frankfurt = data.0.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let needs = requirements(&data.0, frankfurt, &data.1);
        // About 1,200 movements a day.
        assert!((100..=150).contains(&needs.min_stands), "{needs:?}");
        let names: Vec<String> = needs.departures.iter().map(ToString::to_string).collect();
        assert!(names.contains(&"18".to_owned()), "{names:?}");
        let arrivals: Vec<String> = needs.arrivals.iter().map(ToString::to_string).collect();
        assert_eq!(arrivals, ["25L", "25R"]);
        let airfield = data.0.airport_by_ident("EDFE".parse()?).context("EDFE")?;
        let small = requirements(&data.0, airfield, &data.1);
        assert_eq!(small.min_stands, 1);
        assert_eq!(small.arrivals, small.departures);
        assert_ne!(small.arrivals.len(), 0);
        Ok(())
    }

    fn aerocosm_core_test_data() -> Result<(World, TrafficTargets)> {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let world = World::from_json(&std::fs::read_to_string(format!(
            "{root}/world/germany.world.json"
        ))?)?;
        let traffic = TrafficTargets::from_json(&std::fs::read_to_string(format!(
            "{root}/traffic/germany.json"
        ))?)?;
        Ok((world, traffic))
    }
}
