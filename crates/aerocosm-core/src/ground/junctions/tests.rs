use anyhow::{Context, Result};

use super::*;
use crate::{
    ground::{
        AirportGroundDefinition, EdgeDefinition, GroundName, GroundNetwork,
        HoldingPositionDefinition, definition::Point,
    },
    test_data::data,
};

/// A layout near Frankfurt from `nodes` (metres east, north of a corner) and
/// `edges` (from, to, kind, name), with intermediate holds `holds` and
/// runway holds for 25C at `runway_holds`.
fn layout(
    nodes: &[(f64, f64)],
    edges: &[(u32, u32, EdgeKind, Option<&str>)],
    holds: &[u32],
    runway_holds: &[u32],
) -> Result<AirportGround> {
    let points: Vec<Point> = nodes
        .iter()
        .map(|&(east, north)| [50.03 + north / 111_195.0, 8.56 + east / 71_470.0])
        .collect();
    let mut definitions = Vec::with_capacity(edges.len());
    for &(from, to, kind, name) in edges {
        definitions.push(EdgeDefinition {
            from,
            to,
            kind,
            name: name.map(str::parse::<GroundName>).transpose()?,
            oneway: false,
            max_wingspan: None,
            points: Vec::new(),
        });
    }
    AirportGround::new(
        AirportGroundDefinition {
            airport: "EDDF".parse()?,
            nodes: points,
            edges: definitions,
            stands: Vec::new(),
            holding_positions: runway_holds
                .iter()
                .map(|&node| {
                    Ok(HoldingPositionDefinition {
                        node,
                        runway: "25C".parse()?,
                    })
                })
                .collect::<Result<_>>()?,
            exits: Vec::new(),
            intermediate_holds: holds.to_vec(),
            aprons: Vec::new(),
        },
        &data()?.world,
    )
}

const TAXIWAY: EdgeKind = EdgeKind::Taxiway;

/// A cross at node 0: arms east (1) and west (2) of taxiway N, north (3) A,
/// south (4) B, 300 m each.
fn cross(holds: &[u32]) -> Result<AirportGround> {
    layout(
        &[
            (0.0, 0.0),
            (300.0, 0.0),
            (-300.0, 0.0),
            (0.0, 300.0),
            (0.0, -300.0),
        ],
        &[
            (0, 1, TAXIWAY, Some("N")),
            (0, 2, TAXIWAY, Some("N")),
            (0, 3, TAXIWAY, Some("A")),
            (0, 4, TAXIWAY, Some("B")),
        ],
        holds,
        &[],
    )
}

#[test]
fn test_the_through_taxiway_has_right_of_way() -> Result<()> {
    let ground = cross(&[])?;
    let junctions = Junctions::new(&ground);
    assert_eq!(junctions.junctions().len(), 1);
    // Along N, east to west: no hold.
    let (along_n, runs) = junctions.passages(
        &ground,
        Id::new(1),
        &[(Id::new(0), false), (Id::new(1), true)],
        0.0,
    );
    assert_eq!(along_n.len(), 1);
    assert_eq!(along_n[0].junction, 0);
    assert!((along_n[0].entry - 300.0).abs() < 1.0);
    assert_eq!(along_n[0].hold, None);
    assert_eq!(runs.len(), 2);
    assert!(!runs[0].forward && runs[1].forward);
    // Each run starts where the aircraft enters its chain, in the chain's
    // own distance: arm 1 is drawn from the junction out, so it is entered
    // at its far end; arm 2 at its start.
    assert!((runs[0].at - 300.0).abs() < 1.0, "{runs:?}");
    assert!(runs[1].at.abs() < 1e-6);
    assert!((runs[0].position(150.0) - 150.0).abs() < 1.0);
    assert!((runs[1].position(450.0) - 150.0).abs() < 1.0);
    assert!((runs[1].along(150.0) - 450.0).abs() < 1.0);
    // From A, north, onto N: holds 45 m short.
    let (from_a, _) = junctions.passages(
        &ground,
        Id::new(3),
        &[(Id::new(2), false), (Id::new(0), true)],
        0.0,
    );
    let hold = from_a[0].hold.context("a hold")?;
    assert!((hold - 255.0).abs() < 1.0, "{hold}");
    Ok(())
}

#[test]
fn test_a_mapped_hold_gives_way_where_it_is() -> Result<()> {
    // Node 5, 80 m north on A, is a mapped intermediate hold.
    let ground = layout(
        &[
            (0.0, 0.0),
            (300.0, 0.0),
            (-300.0, 0.0),
            (0.0, 300.0),
            (0.0, -300.0),
            (0.0, 80.0),
        ],
        &[
            (0, 1, TAXIWAY, Some("N")),
            (0, 2, TAXIWAY, Some("N")),
            (0, 5, TAXIWAY, Some("A")),
            (5, 3, TAXIWAY, Some("A")),
            (0, 4, TAXIWAY, Some("B")),
        ],
        &[5],
        &[],
    )?;
    let junctions = Junctions::new(&ground);
    let (from_a, _) = junctions.passages(
        &ground,
        Id::new(3),
        &[(Id::new(3), false), (Id::new(2), false), (Id::new(0), true)],
        0.0,
    );
    let hold = from_a[0].hold.context("a hold")?;
    assert!((hold - 220.0).abs() < 1.0, "{hold}");
    Ok(())
}

#[test]
fn test_without_names_the_straight_way_goes_through() -> Result<()> {
    // A T: east-west straight, north arm joining.
    let ground = layout(
        &[(0.0, 0.0), (300.0, 0.0), (-300.0, 0.0), (0.0, 300.0)],
        &[
            (0, 1, TAXIWAY, None),
            (0, 2, TAXIWAY, None),
            (0, 3, TAXIWAY, None),
        ],
        &[],
        &[],
    )?;
    let junctions = Junctions::new(&ground);
    let (straight, _) = junctions.passages(
        &ground,
        Id::new(1),
        &[(Id::new(0), false), (Id::new(1), true)],
        0.0,
    );
    assert_eq!(straight[0].hold, None);
    let (joining, _) = junctions.passages(
        &ground,
        Id::new(3),
        &[(Id::new(2), false), (Id::new(1), true)],
        0.0,
    );
    assert!(joining[0].hold.is_some());
    Ok(())
}

#[test]
fn test_junction_nodes_close_together_are_one_intersection() -> Result<()> {
    // Two junction nodes 20 m apart joined by a short fillet.
    let ground = layout(
        &[
            (0.0, 0.0),
            (20.0, 0.0),
            (-300.0, 0.0),
            (300.0, 0.0),
            (0.0, 300.0),
            (20.0, -300.0),
        ],
        &[
            (0, 1, TAXIWAY, Some("N")),
            (0, 2, TAXIWAY, Some("N")),
            (1, 3, TAXIWAY, Some("N")),
            (0, 4, TAXIWAY, Some("A")),
            (1, 5, TAXIWAY, Some("B")),
        ],
        &[],
        &[],
    )?;
    let junctions = Junctions::new(&ground);
    assert_eq!(junctions.junctions().len(), 1);
    assert_eq!(junctions.junctions()[0].nodes, [Id::new(0), Id::new(1)]);
    let (along, _) = junctions.passages(
        &ground,
        Id::new(2),
        &[(Id::new(1), false), (Id::new(0), true), (Id::new(2), true)],
        0.0,
    );
    assert_eq!(along.len(), 1);
    assert!((along[0].exit - along[0].entry - 20.0).abs() < 1.0);
    Ok(())
}

#[test]
fn test_a_runway_is_entered_from_its_holding_positions() -> Result<()> {
    // Runway 0-1-2; a taxiway from 1 south through hold 3 (60 m) to 4, and
    // one from 2 south to 5 with no hold mapped.
    let ground = layout(
        &[
            (0.0, 0.0),
            (500.0, 0.0),
            (1000.0, 0.0),
            (500.0, -60.0),
            (500.0, -400.0),
            (1000.0, -400.0),
        ],
        &[
            (0, 1, EdgeKind::Runway, None),
            (1, 2, EdgeKind::Runway, None),
            (1, 3, TAXIWAY, None),
            (3, 4, TAXIWAY, None),
            (2, 5, TAXIWAY, None),
        ],
        &[],
        &[3],
    )?;
    let junctions = Junctions::new(&ground);
    assert_eq!(junctions.junctions().len(), 1);
    assert!(junctions.junctions()[0].runway);
    let (mapped, _) = junctions.passages(
        &ground,
        Id::new(4),
        &[(Id::new(3), false), (Id::new(2), false)],
        0.0,
    );
    assert!((mapped[0].hold.context("hold")? - 340.0).abs() < 1.0);
    let (synthesised, _) = junctions.passages(&ground, Id::new(5), &[(Id::new(4), false)], 0.0);
    assert!((synthesised[0].hold.context("hold")? - 310.0).abs() < 1.0);
    Ok(())
}

#[test]
fn test_shipped_hubs_have_compact_junctions() -> Result<()> {
    let world = data()?.world;
    let network = GroundNetwork::from_json(
        include_str!("../../../../../assets/ground/germany.ground.json"),
        &world,
    )?;
    for ident in ["EDDF", "EDDM", "EDDB"] {
        let airport = world.airport_by_ident(ident.parse()?).context("airport")?;
        let ground = network.airport(airport).context("layout")?;
        let junctions = Junctions::new(ground);
        // Complex intersections with many fillets, but each one place.
        let widest = junctions
            .junctions()
            .iter()
            .filter(|j| !j.runway)
            .map(|j| {
                let xy: Vec<[f64; 2]> = j
                    .nodes
                    .iter()
                    .map(|n| ground.nodes()[n.as_usize()].xy)
                    .collect();
                xy.iter()
                    .flat_map(|a| xy.iter().map(move |b| (a[0] - b[0]).hypot(a[1] - b[1])))
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max);
        assert!(widest <= 250.0, "{ident}: a junction {widest:.0} m across");
        assert!(junctions.junctions().len() > 50, "{ident}");
        assert!(!junctions.chains().is_empty());
    }
    Ok(())
}
