//! `OpenStreetMap` airport layouts (Overpass JSON) → the ground file (plan
//! M4 D64–D67).

mod gate;
mod graph;
mod layout;
mod local;
mod names;
mod overpass;
mod query;
mod runways;
mod stands;

use std::fmt;

use aerocosm_core::{
    ground::{FORMAT_VERSION, GroundDefinition, GroundNetwork},
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use anyhow::{Context, Result};

pub use self::query::{ground_airports, overpass_query};

/// Attribution the `ODbL` requires for the ground file (plan M4 D65).
pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";

/// One airport's Overpass download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsmSource {
    /// The airport.
    pub airport: AirportIdent,
    /// The Overpass JSON response.
    pub json: String,
}

/// What the import found at one airport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirportGroundReport {
    /// The airport.
    pub airport: AirportIdent,
    /// Stands kept.
    pub stands: usize,
    /// Stands left out because they do not connect to a runway.
    pub dropped_stands: usize,
    /// Network edges.
    pub edges: usize,
    /// Runway holding positions.
    pub holding_positions: usize,
    /// Runway exits.
    pub exits: usize,
    /// Why the layout failed the quality gate, if it did.
    pub rejected: Option<String>,
}

impl fmt::Display for AirportGroundReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} stands ({} unconnected), {} edges, {} holding positions, {} exits",
            self.airport,
            self.stands,
            self.dropped_stands,
            self.edges,
            self.holding_positions,
            self.exits
        )?;
        match &self.rejected {
            Some(reason) => write!(f, " — rejected: {reason}"),
            None => f.write_str(" — ok"),
        }
    }
}

/// The ground file and what the import did per airport.
#[derive(Debug, Clone)]
pub struct GroundImport {
    /// Contents of the ground file (compact JSON).
    pub ground_json: String,
    /// One report per source, in source order.
    pub reports: Vec<AirportGroundReport>,
    /// The layouts that failed the gate, as ground files of one airport each,
    /// for inspection.
    pub rejected: Vec<(AirportIdent, String)>,
}

/// Imports every source's layout and keeps those passing the quality gate.
/// Airports are written in source order sorted by ident, so the output does
/// not depend on directory listing order.
pub fn import_ground(
    sources: &[OsmSource],
    world: &World,
    traffic: &TrafficTargets,
) -> Result<GroundImport> {
    let mut sources: Vec<&OsmSource> = sources.iter().collect();
    sources.sort_by_key(|source| source.airport);
    let mut airports = Vec::with_capacity(sources.len());
    let mut reports = Vec::with_capacity(sources.len());
    let mut rejected_layouts = Vec::new();
    for source in sources {
        let ident = source.airport;
        let airport = world
            .airport_by_ident(ident)
            .with_context(|| format!("{ident} is not in the world"))?;
        let data = overpass::read_overpass(&source.json)
            .with_context(|| format!("cannot read the OSM data of {ident}"))?;
        let layout = layout::build_layout(&data, world, airport)
            .with_context(|| format!("cannot build the layout of {ident}"))?;
        let requirements = gate::requirements(world, airport, traffic);
        let rejected = gate::check(&layout.definition, &requirements).err();
        let definition = &layout.definition;
        reports.push(AirportGroundReport {
            airport: ident,
            stands: definition.stands.len(),
            dropped_stands: layout.dropped_stands,
            edges: definition.edges.len(),
            holding_positions: definition.holding_positions.len(),
            exits: definition.exits.len(),
            rejected: rejected.clone(),
        });
        if rejected.is_none() {
            airports.push(layout.definition);
        } else {
            let alone = GroundDefinition {
                format_version: FORMAT_VERSION,
                attribution: ATTRIBUTION.to_owned(),
                airports: vec![layout.definition],
            };
            rejected_layouts.push((ident, alone.to_json()?));
        }
    }
    let definition = GroundDefinition {
        format_version: FORMAT_VERSION,
        attribution: ATTRIBUTION.to_owned(),
        airports,
    };
    // The core must accept what the importer writes.
    GroundNetwork::from_definition(definition.clone(), world)
        .context("the imported ground layouts do not validate")?;
    Ok(GroundImport {
        ground_json: definition.to_json()?,
        reports,
        rejected: rejected_layouts,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_report_display() -> Result<()> {
        let mut report = AirportGroundReport {
            airport: "EDDF".parse()?,
            stands: 300,
            dropped_stands: 12,
            edges: 2_000,
            holding_positions: 150,
            exits: 40,
            rejected: None,
        };
        assert_eq!(
            report.to_string(),
            "EDDF: 300 stands (12 unconnected), 2000 edges, 150 holding positions, 40 exits — ok"
        );
        report.rejected = Some("3 stands, 9 needed".to_owned());
        assert!(
            report
                .to_string()
                .ends_with("— rejected: 3 stands, 9 needed")
        );
        Ok(())
    }

    #[test]
    fn test_import_ground_rejects_unknown_airports_and_bad_json() -> Result<()> {
        let world = World::empty();
        let traffic =
            TrafficTargets::from_json(include_str!("../../../../assets/traffic/germany.json"))?;
        let source = OsmSource {
            airport: "EDDF".parse()?,
            json: "{\"elements\": []}".to_owned(),
        };
        assert!(import_ground(&[source], &world, &traffic).is_err());
        let empty = import_ground(&[], &world, &traffic)?;
        assert_eq!(empty.reports.len(), 0);
        assert_eq!(empty.rejected.len(), 0);
        assert!(empty.ground_json.contains("OpenStreetMap"));
        Ok(())
    }
}
