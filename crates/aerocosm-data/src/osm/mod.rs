//! `OpenStreetMap` airport layouts (Overpass JSON) → the ground file (plan
//! M4 D64–D67).

mod gate;
mod graph;
mod layout;
mod local;
mod names;
mod overpass;
mod prune;
mod query;
mod runways;
mod sizes;
mod stands;

use std::fmt;

use aerocosm_core::{
    ground::{FORMAT_VERSION, GroundDefinition, GroundNetwork},
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use anyhow::{Context, Result};

pub use self::{
    query::{ground_airports, overpass_query},
    sizes::{Code, SIZES_VERSION, StandSizes},
};

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
    /// Stands left out because no route leads from them to a departure
    /// runway in use, or to them from an arrival runway.
    pub unroutable_stands: usize,
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
            "{}: {} stands ({} unconnected, {} unroutable), {} edges, {} holding positions, {} \
             exits",
            self.airport,
            self.stands,
            self.dropped_stands,
            self.unroutable_stands,
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
    sizes: &StandSizes,
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
        let mut layout = layout::build_layout(&data, world, airport, sizes)
            .with_context(|| format!("cannot build the layout of {ident}"))?;
        let requirements = gate::requirements(world, airport, traffic);
        let unroutable = prune::prune_stands(&mut layout.definition, world, &requirements)
            .with_context(|| format!("cannot check the stands of {ident}"))?;
        let rejected = gate::check(&layout.definition, &requirements).err();
        let definition = &layout.definition;
        reports.push(AirportGroundReport {
            airport: ident,
            stands: definition.stands.len(),
            dropped_stands: layout.dropped_stands,
            unroutable_stands: unroutable,
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
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_report_display() -> Result<()> {
        let mut report = AirportGroundReport {
            airport: "EDDF".parse()?,
            stands: 300,
            dropped_stands: 12,
            unroutable_stands: 3,
            edges: 2_000,
            holding_positions: 150,
            exits: 40,
            rejected: None,
        };
        assert_eq!(
            report.to_string(),
            "EDDF: 300 stands (12 unconnected, 3 unroutable), 2000 edges, 150 holding positions, \
             40 exits — ok"
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
        assert!(import_ground(&[source], &world, &traffic, &StandSizes::default()).is_err());
        let empty = import_ground(&[], &world, &traffic, &StandSizes::default())?;
        assert_eq!(empty.reports.len(), 0);
        assert_eq!(empty.rejected.len(), 0);
        assert!(empty.ground_json.contains("OpenStreetMap"));
        Ok(())
    }

    /// A world with one airport, EDXX, whose runway 09/27 runs along row 0
    /// of the test grid (≈ 300 m).
    fn grid_world() -> Result<World> {
        World::from_json(
            r#"{"format_version": 2,
              "airports": [{"ident": "EDXX", "name": "Grid", "location": {"lat": 50.0005, "lon": 8.5021},
                "elevation": 300.0, "kind": "regional", "country": "DE"}],
              "runways": [{"airport": "EDXX", "ends": [
                  {"designator": "09", "threshold": {"lat": 50.0, "lon": 8.5}, "true_heading": 90.0, "displaced_threshold": 0.0},
                  {"designator": "27", "threshold": {"lat": 50.0, "lon": 8.5042}, "true_heading": 270.0, "displaced_threshold": 0.0}],
                "length": 300.0, "width": 30.0, "surface": "paved", "geometry": "surveyed"}]}"#,
        )
    }

    /// Runway on row 0, taxiway up column 2 past a holding point at row 8 to
    /// a lane on row 12 with two lead-ins north; a second taxiway at column 28.
    fn grid_source() -> Result<OsmSource> {
        use graph::tests::{data, grid_node, way};
        let tagged = |id: i64, tags: &str| {
            let node = grid_node(id);
            format!("{},\"tags\":{{{tags}}}}}", &node[..node.len() - 1])
        };
        let ways = [
            way(
                1,
                &[0, 2, 28, 30],
                "\"aeroway\":\"runway\",\"ref\":\"09/27\"",
            ),
            way(2, &[2, 802, 1202], "\"aeroway\":\"taxiway\",\"ref\":\"A\""),
            way(3, &[1202, 1210, 1220, 1228], "\"aeroway\":\"taxilane\""),
            way(4, &[28, 828, 1228], "\"aeroway\":\"taxiway\",\"ref\":\"B\""),
            way(
                5,
                &[1210, 1510],
                "\"aeroway\":\"parking_position\",\"ref\":\"1\"",
            ),
            way(
                6,
                &[1220, 1520],
                "\"aeroway\":\"parking_position\",\"ref\":\"2\"",
            ),
        ];
        let extra = [
            tagged(802, "\"aeroway\":\"holding_position\""),
            tagged(828, "\"aeroway\":\"holding_position\""),
        ];
        // Round-trip through the reader's input format.
        let parsed = data(&ways, &extra)?;
        let mut elements: Vec<String> = parsed
            .nodes
            .values()
            .map(|node| {
                let tags: Vec<String> = node
                    .tags
                    .iter()
                    .map(|(k, v)| format!("\"{k}\":\"{v}\""))
                    .collect();
                format!(
                    "{{\"type\":\"node\",\"id\":{},\"lat\":{},\"lon\":{},\"tags\":{{{}}}}}",
                    node.id,
                    node.lat,
                    node.lon,
                    tags.join(",")
                )
            })
            .collect();
        elements.extend_from_slice(&ways);
        Ok(OsmSource {
            airport: "EDXX".parse()?,
            json: format!("{{\"elements\":[{}]}}", elements.join(",")),
        })
    }

    #[test]
    fn test_import_ground_builds_a_layout_end_to_end() -> Result<()> {
        let world = grid_world()?;
        let traffic =
            TrafficTargets::from_json(include_str!("../../../../assets/traffic/germany.json"))?;
        let source = grid_source()?;
        let first = import_ground(
            std::slice::from_ref(&source),
            &world,
            &traffic,
            &StandSizes::default(),
        )?;
        let report = first.reports.first().context("a report")?;
        assert_eq!(report.rejected, None, "{report}");
        assert_eq!((report.stands, report.holding_positions), (2, 2));
        assert!(report.exits >= 2, "{report}");
        let network = GroundNetwork::from_json(&first.ground_json, &world)?;
        let layout = network.airports().first().context("a layout")?;
        let names: Vec<String> = layout
            .stands()
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        assert_eq!(names, ["1", "2"]);
        // Same input, same bytes.
        let second = import_ground(&[source], &world, &traffic, &StandSizes::default())?;
        assert_eq!(first.ground_json, second.ground_json);
        Ok(())
    }
}
