//! `OpenStreetMap` airport layouts (Overpass JSON per airport, or the world's
//! aeroways in OPL) → the ground assets by region (plan M4 D64–D67, M5
//! D116–D119).

mod gate;
mod graph;
mod holds;
mod layout;
mod local;
mod names;
mod opl;
mod overpass;
mod prune;
mod query;
mod runways;
mod stands;

use std::{collections::BTreeMap, fmt};

use aerocosm_core::{
    ground::{
        AirportGroundDefinition, FORMAT_VERSION, GroundDefinition, GroundNetwork, ground_region,
    },
    traffic::TrafficTargets,
    world::{World, codes::AirportIdent},
};
use anyhow::{Context, Result};

pub use self::query::{ground_airports, overpass_query, query_box};
use self::{
    opl::{WorldOsm, read_opl},
    overpass::OsmData,
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

/// Where the OSM data comes from.
#[derive(Debug, Clone, Copy)]
pub enum OsmInput<'a> {
    /// One Overpass download per airport.
    Overpass(&'a [OsmSource]),
    /// The world's aeroways in OPL (plan M5 D116), cut out for each of the
    /// airports (normally [`ground_airports`]).
    Opl {
        /// The OPL text.
        text: &'a str,
        /// The airports to cut out.
        airports: &'a [AirportIdent],
    },
}

/// What the ground import produced.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundImport {
    /// The binary ground asset of each region with a layout (plan M5 D117).
    pub regions: Vec<(Option<char>, Vec<u8>)>,
    /// One report per airport, in ident order.
    pub reports: Vec<AirportGroundReport>,
    /// Layouts that failed the quality gate, as JSON for inspection.
    pub rejected: Vec<(AirportIdent, String)>,
}

/// Builds the layouts of `input`'s airports, checks them against the
/// quality gate and groups those that pass by region.
pub fn import_ground(
    input: OsmInput<'_>,
    world: &World,
    traffic: &TrafficTargets,
) -> Result<GroundImport> {
    let mut sources: Vec<(AirportIdent, OsmData)> = Vec::new();
    match input {
        OsmInput::Overpass(downloads) => {
            for source in downloads {
                let data = overpass::read_overpass(&source.json)
                    .with_context(|| format!("cannot read the OSM data of {}", source.airport))?;
                sources.push((source.airport, data));
            }
        }
        OsmInput::Opl { text, airports } => {
            let osm = WorldOsm::new(read_opl(text).context("cannot read the OPL file")?);
            for &airport in airports {
                sources.push((airport, osm.within(query_box(world, airport)?)));
            }
        }
    }
    sources.sort_by_key(|(airport, _)| *airport);
    let mut regions: BTreeMap<Option<char>, Vec<AirportGroundDefinition>> = BTreeMap::new();
    let mut reports = Vec::with_capacity(sources.len());
    let mut rejected_layouts = Vec::new();
    for (ident, data) in sources {
        let airport = world
            .airport_by_ident(ident)
            .with_context(|| format!("{ident} is not in the world"))?;
        let requirements = gate::requirements(world, airport, traffic);
        let mut layout = layout::build_layout(&data, world, airport, &requirements.departures)
            .with_context(|| format!("cannot build the layout of {ident}"))?;
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
            regions
                .entry(ground_region(ident))
                .or_default()
                .push(layout.definition);
        } else if !definition.edges.is_empty() {
            let alone = GroundDefinition {
                format_version: FORMAT_VERSION,
                attribution: ATTRIBUTION.to_owned(),
                airports: vec![layout.definition],
            };
            rejected_layouts.push((ident, alone.to_json()?));
        }
    }
    let mut assets = Vec::with_capacity(regions.len());
    for (region, airports) in regions {
        let definition = GroundDefinition {
            format_version: FORMAT_VERSION,
            attribution: ATTRIBUTION.to_owned(),
            airports,
        };
        // The core must accept what the importer writes.
        GroundNetwork::from_definition(definition.clone(), world)
            .with_context(|| format!("the ground layouts of region {region:?} do not validate"))?;
        assets.push((region, definition.to_asset()?));
    }
    Ok(GroundImport {
        regions: assets,
        reports,
        rejected: rejected_layouts,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_shipped_region_assets_load_and_cover_the_world() -> Result<()> {
        let (world, _) = crate::shipped_model()?;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ground");
        let mut regions = 0;
        let mut airports = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".ground.bin") {
                continue;
            }
            let definition = GroundDefinition::from_asset(&std::fs::read(&path)?)
                .with_context(|| name.to_owned())?;
            assert_eq!(definition.attribution, ATTRIBUTION, "{name}");
            for layout in &definition.airports {
                // Each airport in the file of its region (plan M5 D117).
                let file = aerocosm_core::ground::ground_file(ground_region(layout.airport));
                assert_eq!(file, name, "{}", layout.airport);
                airports.push(layout.airport.to_string());
            }
            GroundNetwork::from_definition(definition, &world).with_context(|| name.to_owned())?;
            regions += 1;
        }
        assert!(regions >= 15, "{regions} regions");
        assert!(airports.len() > 600, "{} airports", airports.len());
        // A hub on every continent.
        for hub in [
            "EDDF", "EGLL", "KATL", "SBGR", "FAOR", "OMDB", "RJTT", "YSSY",
        ] {
            assert!(airports.contains(&hub.to_owned()), "{hub}");
        }
        Ok(())
    }

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
        let (_, traffic) = crate::shipped_model()?;
        let source = OsmSource {
            airport: "EDDF".parse()?,
            json: "{\"elements\": []}".to_owned(),
        };
        let sources = [source];
        assert!(import_ground(OsmInput::Overpass(&sources), &world, &traffic).is_err());
        let empty = import_ground(OsmInput::Overpass(&[]), &world, &traffic)?;
        assert_eq!(empty.reports.len(), 0);
        assert_eq!(empty.rejected.len(), 0);
        assert_eq!(empty.regions, []);
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
        let (_, traffic) = crate::shipped_model()?;
        let sources = [grid_source()?];
        let first = import_ground(OsmInput::Overpass(&sources), &world, &traffic)?;
        let report = first.reports.first().context("a report")?;
        assert_eq!(report.rejected, None, "{report}");
        assert_eq!((report.stands, report.holding_positions), (2, 2));
        assert!(report.exits >= 2, "{report}");
        let (region, bytes) = first.regions.first().context("a region")?;
        assert_eq!(*region, Some('E'));
        let network = GroundNetwork::from_definition(GroundDefinition::from_asset(bytes)?, &world)?;
        let layout = network.airports().first().context("a layout")?;
        let names: Vec<String> = layout
            .stands()
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        assert_eq!(names, ["1", "2"]);
        // Same input, same bytes.
        let second = import_ground(OsmInput::Overpass(&sources), &world, &traffic)?;
        assert_eq!(first.regions, second.regions);
        Ok(())
    }

    /// The grid source as OPL, as `osmium cat -f opl` writes it.
    fn grid_opl() -> Result<String> {
        let source = grid_source()?;
        let data = overpass::read_overpass(&source.json)?;
        let tags = |tags: &overpass::Tags| -> String {
            tags.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut lines: Vec<String> = data
            .nodes
            .values()
            .map(|n| format!("n{} v1 T{} x{} y{}", n.id, tags(&n.tags), n.lon, n.lat))
            .collect();
        lines.sort();
        for way in &data.ways {
            let nodes: Vec<String> = way.nodes.iter().map(|n| format!("n{n}")).collect();
            lines.push(format!(
                "w{} v1 T{} N{}",
                way.id,
                tags(&way.tags),
                nodes.join(",")
            ));
        }
        Ok(lines.join("\n"))
    }

    #[test]
    fn test_import_ground_cuts_airports_from_opl() -> Result<()> {
        let world = grid_world()?;
        let (_, traffic) = crate::shipped_model()?;
        let overpass = import_ground(OsmInput::Overpass(&[grid_source()?]), &world, &traffic)?;
        let text = grid_opl()?;
        let airports = ["EDXX".parse()?];
        let opl = import_ground(
            OsmInput::Opl {
                text: &text,
                airports: &airports,
            },
            &world,
            &traffic,
        )?;
        // The same layout either way.
        assert_eq!(opl.regions, overpass.regions);
        assert_eq!(opl.reports, overpass.reports);
        Ok(())
    }
}
