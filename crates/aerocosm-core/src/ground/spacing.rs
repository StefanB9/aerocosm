//! Which nodes lie too close together for two aircraft (plan M4 D70): ground
//! control holds them together, so aircraft waiting at neighbouring nodes do
//! not overlap.

use std::collections::HashMap;

use super::{AirportGround, Node};
use crate::Id;

impl AirportGround {
    /// Per node, the other nodes within `radius` metres.
    pub(crate) fn close_nodes(&self, radius: f64) -> Vec<Vec<Id<Node>>> {
        let cell = |xy: [f64; 2]| {
            // SAFETY: airport coordinates are a few kilometres from the origin.
            #[allow(clippy::cast_possible_truncation)]
            let key = (
                (xy[0] / radius).floor() as i64,
                (xy[1] / radius).floor() as i64,
            );
            key
        };
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (index, node) in self.nodes().iter().enumerate() {
            grid.entry(cell(node.xy)).or_default().push(index);
        }
        self.nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let (x, y) = cell(node.xy);
                let mut near = Vec::new();
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for &other in grid.get(&(x + dx, y + dy)).map_or(&[][..], Vec::as_slice) {
                            let xy = self.nodes()[other].xy;
                            if other != index
                                && (xy[0] - node.xy[0]).hypot(xy[1] - node.xy[1]) <= radius
                                && let Ok(id) = Id::from_usize(other)
                            {
                                near.push(id);
                            }
                        }
                    }
                }
                near.sort_unstable();
                near
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use crate::{ground::GroundNetwork, test_data::data};

    #[test]
    fn test_close_nodes_match_a_full_search_at_frankfurt() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let ground = network.airport(frankfurt).context("layout")?;
        let near = ground.close_nodes(45.0);
        assert_eq!(near.len(), ground.nodes().len());
        let nodes = ground.nodes();
        let mut pairs = 0;
        for (index, list) in near.iter().enumerate() {
            let expected: Vec<usize> = (0..nodes.len())
                .filter(|&other| {
                    other != index
                        && (nodes[other].xy[0] - nodes[index].xy[0])
                            .hypot(nodes[other].xy[1] - nodes[index].xy[1])
                            <= 45.0
                })
                .collect();
            let got: Vec<usize> = list.iter().map(|id| id.as_usize()).collect();
            assert_eq!(got, expected, "node {index}");
            pairs += got.len();
        }
        assert_ne!(pairs, 0);
        Ok(())
    }
}
