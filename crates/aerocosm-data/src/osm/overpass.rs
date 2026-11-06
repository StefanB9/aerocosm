//! The subset of Overpass API JSON output the ground import reads: nodes with
//! coordinates and ways with node lists, both with optional tags.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use serde::Deserialize;

/// OSM tags of an element.
pub(crate) type Tags = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
struct Response {
    elements: Vec<Element>,
    /// Set when the server stopped early, e.g. on a timeout.
    #[serde(default)]
    remark: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Element {
    Node {
        id: i64,
        lat: f64,
        lon: f64,
        #[serde(default)]
        tags: Tags,
    },
    Way {
        id: i64,
        nodes: Vec<i64>,
        #[serde(default)]
        tags: Tags,
    },
    // Relations (e.g. multipolygon aprons) are not used.
    Relation {},
}

/// A tagged OSM node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OsmNode {
    pub(crate) id: i64,
    pub(crate) lat: f64,
    pub(crate) lon: f64,
    pub(crate) tags: Tags,
}

/// An OSM way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OsmWay {
    pub(crate) id: i64,
    pub(crate) nodes: Vec<i64>,
    pub(crate) tags: Tags,
}

impl OsmWay {
    /// The way's `aeroway` tag.
    pub(crate) fn aeroway(&self) -> Option<&str> {
        self.tags.get("aeroway").map(String::as_str)
    }

    /// Whether the way is closed and tagged as an area.
    pub(crate) fn is_area(&self) -> bool {
        self.nodes.len() > 2
            && self.nodes.first() == self.nodes.last()
            && self.tags.get("area").is_some_and(|v| v == "yes")
    }
}

/// One airport's download: every node by id, ways in file order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct OsmData {
    pub(crate) nodes: HashMap<i64, OsmNode>,
    pub(crate) ways: Vec<OsmWay>,
}

impl OsmData {
    /// Coordinates of node `id`, as `[lat, lon]`.
    pub(crate) fn position(&self, id: i64) -> Option<[f64; 2]> {
        self.nodes.get(&id).map(|node| [node.lat, node.lon])
    }
}

/// Parses an Overpass JSON response. Ways are sorted by id so the import does
/// not depend on the order a server returns them in.
pub(crate) fn read_overpass(text: &str) -> Result<OsmData> {
    let response: Response = serde_json::from_str(text).context("malformed Overpass JSON")?;
    if let Some(remark) = response.remark {
        anyhow::bail!("incomplete Overpass response: {remark}");
    }
    let mut data = OsmData::default();
    for element in response.elements {
        match element {
            Element::Node { id, lat, lon, tags } => {
                // `out skel` repeats tagged nodes without tags; keep the tags.
                let entry = data.nodes.entry(id).or_insert_with(|| OsmNode {
                    id,
                    lat,
                    lon,
                    tags: Tags::new(),
                });
                if !tags.is_empty() {
                    entry.tags = tags;
                }
            }
            Element::Way { id, nodes, tags } => data.ways.push(OsmWay { id, nodes, tags }),
            Element::Relation {} => {}
        }
    }
    data.ways.sort_by_key(|way| way.id);
    data.ways.dedup_by_key(|way| way.id);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    const SAMPLE: &str = r#"{
      "version": 0.6,
      "generator": "Overpass API",
      "osm3s": {"timestamp_osm_base": "2026-11-06T00:00:00Z"},
      "elements": [
        {"type": "way", "id": 20, "nodes": [1, 2], "tags": {"aeroway": "taxiway", "ref": "N7"}},
        {"type": "node", "id": 2, "lat": 50.1, "lon": 8.6,
         "tags": {"aeroway": "holding_position"}},
        {"type": "way", "id": 10, "nodes": [2, 3, 4, 2],
         "tags": {"aeroway": "taxiway", "area": "yes"}},
        {"type": "relation", "id": 5, "members": [], "tags": {}},
        {"type": "node", "id": 1, "lat": 50.0, "lon": 8.5},
        {"type": "node", "id": 2, "lat": 50.1, "lon": 8.6},
        {"type": "way", "id": 20, "nodes": [1, 2], "tags": {"aeroway": "taxiway", "ref": "N7"}}
      ]
    }"#;

    #[test]
    fn test_read_overpass_collects_nodes_and_sorted_ways() -> Result<()> {
        let data = read_overpass(SAMPLE)?;
        assert_eq!(data.ways.iter().map(|w| w.id).collect::<Vec<_>>(), [10, 20]);
        assert_eq!(data.position(1), Some([50.0, 8.5]));
        assert_eq!(data.position(9), None);
        // The bare repeat of node 2 keeps its tags.
        let holding = data.nodes.get(&2).context("node 2")?;
        assert_eq!(
            holding.tags.get("aeroway").map(String::as_str),
            Some("holding_position")
        );
        assert_eq!(data.ways[1].aeroway(), Some("taxiway"));
        assert!(data.ways[0].is_area());
        assert!(!data.ways[1].is_area());
        Ok(())
    }

    #[test]
    fn test_read_overpass_rejects_other_json() {
        assert!(read_overpass("{\"remark\": \"runtime error\"}").is_err());
        assert!(read_overpass("<html>504</html>").is_err());
        assert!(read_overpass("{\"elements\": [], \"remark\": \"timed out\"}").is_err());
    }
}
