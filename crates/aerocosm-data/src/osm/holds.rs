//! Runway holding positions by rule (plan M5 D126): where `OpenStreetMap`
//! maps none for a runway end, one is placed on each taxiway that joins the
//! runway in that end's half, at the ICAO Annex 14 distance from the
//! centreline. Mapped holding positions always win.

use aerocosm_core::world::RunwayDesignator;

use super::{
    local::{Plane, Xy, distance},
    overpass::{OsmData, OsmNode, Tags},
    runways::{HOLDING_REACH, LocalRunway},
};

/// Annex 14 (table 3-2) distance of a runway holding position from the
/// centreline of an instrument runway, by runway length: code 4 (from
/// 1,800 m), code 3 (from 1,200 m), smaller.
fn holding_distance(length: f64) -> f64 {
    if length >= 1_800.0 {
        90.0
    } else if length >= 1_200.0 {
        75.0
    } else {
        40.0
    }
}

/// Whether `node` is a mapped runway holding position.
fn is_runway_hold(node: &OsmNode) -> bool {
    node.tags
        .get("aeroway")
        .is_some_and(|a| a == "holding_position")
        && node
            .tags
            .get("holding_position:type")
            .is_none_or(|kind| kind == "runway")
}

/// The runway end (0 or 1) whose half of `runway` the point `p` lies in.
fn half(runway: &LocalRunway, p: Xy) -> usize {
    let (along, _) = runway.locate(p, 0);
    usize::from(along > runway.length() / 2.0)
}

/// Holding positions this close together, or to a taxiway node, are one,
/// m.
const SAME_SPOT: f64 = 5.0;

/// Where the taxiway of `nodes`, leaving `runway` at node `start`, is
/// `hold` off the centreline, each way along it: (index to insert at,
/// point).
fn spots(
    nodes: &[i64],
    at: &impl Fn(i64) -> Option<Xy>,
    runway: &LocalRunway,
    start: usize,
    hold: f64,
) -> Vec<(usize, Xy)> {
    let mut found = Vec::new();
    for forward in [true, false] {
        let mut here = start;
        loop {
            let next = if forward {
                here + 1
            } else {
                here.wrapping_sub(1)
            };
            let (Some(near), Some(far)) = (
                nodes.get(here).and_then(|&n| at(n)),
                nodes.get(next).and_then(|&n| at(n)),
            ) else {
                break;
            };
            let (_, off_near) = runway.locate(near, 0);
            let (_, off_far) = runway.locate(far, 0);
            if off_far < off_near {
                break;
            }
            if off_far >= hold {
                let share = ((hold - off_near) / (off_far - off_near)).clamp(0.0, 1.0);
                found.push((
                    here.max(next),
                    [
                        near[0] + share * (far[0] - near[0]),
                        near[1] + share * (far[1] - near[1]),
                    ],
                ));
                break;
            }
            here = next;
        }
    }
    found
}

/// `data` with a holding position placed by rule for every runway end of
/// `departures` (those in use) that has none mapped; the input itself if
/// every one has.
pub(crate) fn place_holds(
    data: &OsmData,
    plane: &Plane,
    runways: &[LocalRunway],
    departures: &[RunwayDesignator],
) -> OsmData {
    let at = |id: i64| data.position(id).map(|p| plane.project(p));
    // Ends served by a mapped holding position.
    // Ends not in use count as served: nobody departs there.
    let mut served: Vec<[bool; 2]> = runways
        .iter()
        .map(|runway| runway.ends.map(|(end, _)| !departures.contains(&end)))
        .collect();
    for node in data.nodes.values().filter(|node| is_runway_hold(node)) {
        let p = plane.project([node.lat, node.lon]);
        for (r, runway) in runways.iter().enumerate() {
            let (along, off) = runway.locate(p, 0);
            if off <= HOLDING_REACH
                && (-HOLDING_REACH..=runway.length() + HOLDING_REACH).contains(&along)
            {
                served[r][half(runway, p)] = true;
            }
        }
    }
    if served.iter().all(|ends| ends[0] && ends[1]) {
        return data.clone();
    }
    let mut out = data.clone();
    // Taxiway nodes, to place a holding position on one that is already
    // there.
    let mut nodes: Vec<(i64, Xy)> = data
        .ways
        .iter()
        .filter(|way| matches!(way.aeroway(), Some("taxiway" | "taxilane")))
        .flat_map(|way| way.nodes.iter().copied())
        .filter_map(|id| Some((id, at(id)?)))
        .collect();
    nodes.sort_by_key(|(id, _)| *id);
    nodes.dedup_by_key(|(id, _)| *id);
    let mut next_id = data.nodes.keys().copied().min().unwrap_or(0).min(0) - 1;
    for way in &mut out.ways {
        if !matches!(way.aeroway(), Some("taxiway" | "taxilane")) {
            continue;
        }
        // Insertions shift the indices: collect first, insert from the back.
        let mut inserts: Vec<(usize, Xy)> = Vec::new();
        for (r, runway) in runways.iter().enumerate() {
            let hold = holding_distance(runway.length());
            for (start, &id) in way.nodes.iter().enumerate() {
                let Some(point) = at(id) else { continue };
                if !runway.contains(point) || served[r][half(runway, point)] {
                    continue;
                }
                inserts.extend(spots(&way.nodes, &at, runway, start, hold));
            }
        }
        inserts.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        inserts.dedup_by_key(|(at, _)| *at);
        for (index, point) in inserts {
            // Where taxiways meet or overlap, one holding position.
            let id = if let Some(&(id, _)) = nodes
                .iter()
                .find(|(_, xy)| distance(*xy, point) <= SAME_SPOT)
            {
                id
            } else {
                let id = next_id;
                next_id -= 1;
                nodes.push((id, point));
                let [lat, lon] = plane.unproject(point);
                out.nodes.insert(
                    id,
                    OsmNode {
                        id,
                        lat,
                        lon,
                        tags: Tags::new(),
                    },
                );
                id
            };
            if let Some(node) = out.nodes.get_mut(&id) {
                node.tags
                    .insert("aeroway".to_owned(), "holding_position".to_owned());
            }
            if !way.nodes.contains(&id) {
                way.nodes.insert(index, id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::osm::overpass::OsmWay;

    fn node(plane: &Plane, id: i64, xy: Xy, tags: &[(&str, &str)]) -> OsmNode {
        let [lat, lon] = plane.unproject(xy);
        OsmNode {
            id,
            lat,
            lon,
            tags: tags
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    fn in_use() -> Result<Vec<RunwayDesignator>> {
        Ok(vec!["09".parse()?, "27".parse()?])
    }

    /// A 2,000 m runway along x, and a taxiway leaving it at 500 m, north
    /// to 300 m off the centreline.
    fn sample(plane: &Plane, mapped: bool) -> Result<(OsmData, LocalRunway)> {
        let mut data = OsmData::default();
        for (id, xy) in [(1, [500.0, 0.0]), (2, [500.0, 300.0])] {
            data.nodes.insert(id, node(plane, id, xy, &[]));
        }
        if mapped {
            let hold = node(plane, 3, [500.0, 90.0], &[("aeroway", "holding_position")]);
            data.nodes.insert(3, hold);
        }
        data.ways.push(OsmWay {
            id: 10,
            nodes: if mapped { vec![1, 3, 2] } else { vec![1, 2] },
            tags: [("aeroway".to_owned(), "taxiway".to_owned())].into(),
        });
        let runway = LocalRunway {
            ends: [("09".parse()?, [0.0, 0.0]), ("27".parse()?, [2_000.0, 0.0])],
        };
        Ok((data, runway))
    }

    #[test]
    fn test_holds_are_placed_at_the_annex_14_distance() -> Result<()> {
        let plane = Plane::new([50.0, 8.5]);
        let (data, runway) = sample(&plane, false)?;
        let placed = place_holds(&data, &plane, &[runway], &in_use()?);
        let way = placed.ways.first().context("the taxiway")?;
        assert_eq!(way.nodes.len(), 3);
        let hold = placed.nodes.get(&way.nodes[1]).context("the hold")?;
        assert!(is_runway_hold(hold));
        let (along, off) = runway.locate(plane.project([hold.lat, hold.lon]), 0);
        assert!((off - 90.0).abs() < 0.5, "{off}");
        assert!((along - 500.0).abs() < 0.5, "{along}");
        Ok(())
    }

    #[test]
    fn test_mapped_holds_win() -> Result<()> {
        let plane = Plane::new([50.0, 8.5]);
        let (data, runway) = sample(&plane, true)?;
        // 09's half has a mapped hold; 27's has no taxiway: nothing placed.
        assert_eq!(place_holds(&data, &plane, &[runway], &in_use()?), data);
        Ok(())
    }

    #[test]
    fn test_taxiways_on_one_line_share_one_hold() -> Result<()> {
        let plane = Plane::new([50.0, 8.5]);
        let (mut data, runway) = sample(&plane, false)?;
        // A second taxiway drawn over the first, with its own nodes.
        for (id, xy) in [(4, [500.0, 0.0]), (5, [500.0, 300.0])] {
            data.nodes.insert(id, node(&plane, id, xy, &[]));
        }
        data.ways.push(OsmWay {
            id: 11,
            nodes: vec![4, 5],
            tags: [("aeroway".to_owned(), "taxilane".to_owned())].into(),
        });
        let placed = place_holds(&data, &plane, &[runway], &in_use()?);
        let holds: std::collections::HashSet<i64> = placed
            .ways
            .iter()
            .flat_map(|way| way.nodes.iter().copied())
            .filter(|id| placed.nodes.get(id).is_some_and(is_runway_hold))
            .collect();
        assert_eq!(holds.len(), 1, "{holds:?}");
        Ok(())
    }

    #[test]
    fn test_holds_only_for_runway_ends_in_use() -> Result<()> {
        let plane = Plane::new([50.0, 8.5]);
        let (data, runway) = sample(&plane, false)?;
        // Only 27 departs: the taxiway in 09's half gets no hold.
        assert_eq!(
            place_holds(&data, &plane, &[runway], &["27".parse()?]),
            data
        );
        Ok(())
    }

    #[test]
    fn test_holding_distance_by_runway_code() {
        assert!((holding_distance(3_000.0) - 90.0).abs() < 1e-9);
        assert!((holding_distance(1_500.0) - 75.0).abs() < 1e-9);
        assert!((holding_distance(800.0) - 40.0).abs() < 1e-9);
    }
}
