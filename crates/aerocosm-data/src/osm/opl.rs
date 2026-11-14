//! OSM's line-based OPL text format (as `osmium cat -f opl` writes it): the
//! world's aeroway features extracted from the planet file (plan M5 D116),
//! and each airport's part of them.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use super::{
    overpass::{OsmData, OsmNode, OsmWay, Tags},
    query::{NODE_AEROWAYS, WAY_AEROWAYS},
};

/// Grid cells for finding the ways near an airport, degrees.
const CELL: f64 = 0.1;

/// Reads OPL `text`: nodes with coordinates, ways with node lists, both with
/// tags; relations are skipped. Ways are sorted by id.
pub(crate) fn read_opl(text: &str) -> Result<OsmData> {
    let mut data = OsmData::default();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        let Some(kind) = line.chars().next() else {
            continue;
        };
        if kind == 'r' {
            continue;
        }
        let parsed = parse_line(line).with_context(|| format!("OPL line {}", number + 1));
        match parsed? {
            Element::Node(node) => {
                data.nodes.insert(node.id, node);
            }
            Element::Way(way) => data.ways.push(way),
        }
    }
    data.ways.sort_by_key(|way| way.id);
    data.ways.dedup_by_key(|way| way.id);
    Ok(data)
}

enum Element {
    Node(OsmNode),
    Way(OsmWay),
}

fn parse_line(line: &str) -> Result<Element> {
    let mut fields = line.split(' ');
    let head = fields.next().context("an empty line")?;
    let (kind, id) = head.split_at(1);
    let id: i64 = id.parse().with_context(|| format!("id {id}"))?;
    let (mut tags, mut nodes, mut lat, mut lon) = (Tags::new(), Vec::new(), None, None);
    for field in fields {
        let Some(key) = field.chars().next() else {
            continue;
        };
        let value = &field[1..];
        match key {
            'T' if !value.is_empty() => tags = parse_tags(value)?,
            'N' if !value.is_empty() => {
                nodes = value
                    .split(',')
                    .map(|n| -> Result<i64> {
                        Ok(n.strip_prefix('n').context("a node reference")?.parse()?)
                    })
                    .collect::<Result<_>>()
                    .context("node list")?;
            }
            'x' if !value.is_empty() => lon = Some(value.parse::<f64>().context("x")?),
            'y' if !value.is_empty() => lat = Some(value.parse::<f64>().context("y")?),
            _ => {}
        }
    }
    match kind {
        "n" => {
            let (Some(lat), Some(lon)) = (lat, lon) else {
                bail!("node {id} without coordinates");
            };
            Ok(Element::Node(OsmNode { id, lat, lon, tags }))
        }
        "w" => Ok(Element::Way(OsmWay { id, nodes, tags })),
        other => bail!("unknown element type {other}"),
    }
}

/// `key=value` pairs separated by commas, with `%xx%` escapes (hexadecimal
/// Unicode code points) for the separators and other special characters.
fn parse_tags(text: &str) -> Result<Tags> {
    text.split(',')
        .map(|pair| {
            let (key, value) = pair.split_once('=').context("a tag without =")?;
            Ok((unescape(key)?, unescape(value)?))
        })
        .collect()
}

fn unescape(text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut parts = text.split('%');
    out.push_str(parts.next().unwrap_or_default());
    // After the first part, parts alternate: escape, literal, escape, …
    let mut escaped = true;
    for part in parts {
        if escaped {
            let code = u32::from_str_radix(part, 16).with_context(|| format!("escape %{part}%"))?;
            out.push(char::from_u32(code).context("escaped code point")?);
        } else {
            out.push_str(part);
        }
        escaped = !escaped;
    }
    if !escaped {
        bail!("unterminated escape in {text}");
    }
    Ok(out)
}

/// The ways of a world-wide [`OsmData`] by grid cell, for cutting out
/// airports.
pub(crate) struct WorldOsm {
    data: OsmData,
    /// Ways by the cells their nodes lie in.
    cells: HashMap<(i32, i32), Vec<usize>>,
    /// Tagged nodes the import reads, by cell.
    tagged: HashMap<(i32, i32), Vec<i64>>,
}

impl WorldOsm {
    /// Indexes the ways of `data` by the cells their nodes lie in.
    pub(crate) fn new(data: OsmData) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, way) in data.ways.iter().enumerate() {
            let mut seen: Vec<(i32, i32)> = Vec::new();
            for &node in &way.nodes {
                if let Some([lat, lon]) = data.position(node) {
                    let key = cell(lat, lon);
                    if !seen.contains(&key) {
                        seen.push(key);
                        cells.entry(key).or_default().push(index);
                    }
                }
            }
        }
        let mut tagged: HashMap<(i32, i32), Vec<i64>> = HashMap::new();
        for node in data.nodes.values() {
            if reads(node) {
                tagged
                    .entry(cell(node.lat, node.lon))
                    .or_default()
                    .push(node.id);
            }
        }
        for ids in tagged.values_mut() {
            ids.sort_unstable();
        }
        Self {
            data,
            cells,
            tagged,
        }
    }

    /// The part inside `bbox` (south, west, north, east) that an Overpass
    /// query for the airport returns: every way of the aeroways the import
    /// reads with a node inside, with all its nodes, and the nodes of those
    /// aeroways inside; other tags on the nodes are dropped.
    pub(crate) fn within(&self, bbox: [f64; 4]) -> OsmData {
        let [south, west, north, east] = bbox;
        let inside =
            |lat: f64, lon: f64| (south..=north).contains(&lat) && (west..=east).contains(&lon);
        let (low, high) = (cell(south, west), cell(north, east));
        let mut indices: Vec<usize> = Vec::new();
        for y in low.0..=high.0 {
            for x in low.1..=high.1 {
                if let Some(list) = self.cells.get(&(y, x)) {
                    indices.extend(list);
                }
            }
        }
        indices.sort_unstable();
        indices.dedup();
        let mut out = OsmData::default();
        for index in indices {
            let way = &self.data.ways[index];
            if !way.aeroway().is_some_and(|a| WAY_AEROWAYS.contains(&a)) {
                continue;
            }
            let hit = way.nodes.iter().any(|&n| {
                self.data
                    .position(n)
                    .is_some_and(|[lat, lon]| inside(lat, lon))
            });
            if !hit {
                continue;
            }
            for &n in &way.nodes {
                if let Some(node) = self.data.nodes.get(&n) {
                    out.nodes.entry(n).or_insert_with(|| OsmNode {
                        tags: Tags::new(),
                        ..node.clone()
                    });
                }
            }
            out.ways.push(way.clone());
        }
        for y in low.0..=high.0 {
            for x in low.1..=high.1 {
                for id in self.tagged.get(&(y, x)).into_iter().flatten() {
                    if let Some(node) = self.data.nodes.get(id)
                        && inside(node.lat, node.lon)
                    {
                        out.nodes.insert(node.id, node.clone());
                    }
                }
            }
        }
        out
    }
}

/// Whether the import reads `node`'s tags.
fn reads(node: &OsmNode) -> bool {
    node.tags
        .get("aeroway")
        .is_some_and(|a| NODE_AEROWAYS.contains(&a.as_str()))
}

// SAFETY: latitudes and longitudes over CELL are far inside i32.
#[allow(clippy::cast_possible_truncation)]
fn cell(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;

    const SAMPLE: &str = "\
n1 v3 dV c1 t2020-01-01T00:00:00Z i1 utest T x8.5 y50
n2 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Taeroway=holding_position x8.6 y50.1
n3 v1 dV c1 t2020-01-01T00:00:00Z i1 utest T x9.5 y51
w20 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Taeroway=taxiway,ref=N7 Nn1,n2
w10 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Taeroway=parking_position,name=Vorfeld%20%V%2c%1 Nn3,n2
n4 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Taeroway=windsock x8.51 y50.01
w30 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Taeroway=hangar Nn1,n4
r5 v1 dV c1 t2020-01-01T00:00:00Z i1 utest Ttype=multipolygon Mw10@outer
";

    #[test]
    fn test_read_opl_collects_nodes_and_sorted_ways() -> Result<()> {
        let data = read_opl(SAMPLE)?;
        assert_eq!(
            data.ways.iter().map(|w| w.id).collect::<Vec<_>>(),
            [10, 20, 30]
        );
        assert_eq!(data.position(1), Some([50.0, 8.5]));
        assert_eq!(data.ways[1].nodes, [1, 2]);
        assert_eq!(data.ways[1].aeroway(), Some("taxiway"));
        let holding = data.nodes.get(&2).context("node 2")?;
        assert_eq!(
            holding.tags.get("aeroway").map(String::as_str),
            Some("holding_position")
        );
        // Escaped space and comma.
        assert_eq!(
            data.ways[0].tags.get("name").map(String::as_str),
            Some("Vorfeld V,1")
        );
        Ok(())
    }

    #[test]
    fn test_read_opl_rejects_broken_lines() {
        assert!(read_opl("n1 v1 x8.5").is_err(), "no latitude");
        assert!(read_opl("wx v1").is_err(), "bad id");
        assert!(
            read_opl("w1 Taeroway=taxiway Nn1,2").is_err(),
            "bad reference"
        );
        assert!(read_opl("w1 Tname=a%2").is_err(), "unterminated escape");
        assert!(read_opl("q1 v1").is_err(), "unknown type");
    }

    #[test]
    fn test_within_takes_ways_with_a_node_inside_whole() -> Result<()> {
        let world = WorldOsm::new(read_opl(SAMPLE)?);
        // Round node 1 only: way 20 (nodes 1, 2) whole, not way 10; not the
        // hangar or the windsock, which the import does not read.
        let near = world.within([49.95, 8.45, 50.05, 8.55]);
        assert_eq!(near.ways.iter().map(|w| w.id).collect::<Vec<_>>(), [20]);
        assert!(near.nodes.contains_key(&1) && near.nodes.contains_key(&2));
        assert!(!near.nodes.contains_key(&3) && !near.nodes.contains_key(&4));
        // The holding position outside the box comes without its tags, as
        // from Overpass's recursion.
        let outside = near.nodes.get(&2).context("node 2")?;
        assert!(outside.tags.is_empty());
        // Round node 2: both ways, and the tagged node itself.
        let both = world.within([50.05, 8.55, 50.15, 8.65]);
        assert_eq!(both.ways.len(), 2);
        assert_eq!(both.nodes.len(), 3);
        let holding = both.nodes.get(&2).context("node 2")?;
        assert!(holding.tags.contains_key("aeroway"));
        assert_eq!(world.within([0.0, 0.0, 1.0, 1.0]).ways, []);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Escaped tags read back as written.
        #[test]
        fn test_tags_round_trip_through_escapes(value in "[a-zA-Z0-9 ,=%äö_-]{0,20}") {
            let escaped: String = value
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c.to_string()
                    } else {
                        format!("%{:x}%", u32::from(c))
                    }
                })
                .collect();
            let tags = parse_tags(&format!("name={escaped}"))
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(tags.get("name").map(String::as_str), Some(value.as_str()));
        }
    }
}
