//! Which nodes and points lie too close together for two aircraft (plan M4
//! D70, D80): ground control holds them together, so aircraft waiting at
//! neighbouring places do not overlap.

use std::collections::HashMap;

use super::{AirportGround, Edge, Node};
use crate::Id;

impl AirportGround {
    /// Per node, the other nodes within `radius` metres.
    pub(crate) fn close_nodes(&self, radius: f64) -> Vec<Vec<Id<Node>>> {
        let xy: Vec<[f64; 2]> = self.nodes().iter().map(|node| node.xy).collect();
        close_points(&xy, radius)
            .into_iter()
            .map(|near| near.into_iter().map(Id::new).collect())
            .collect()
    }

    /// The point `distance` metres along `edge`, in the flat frame of the
    /// node positions.
    pub(crate) fn edge_point(&self, edge: &Edge, distance: f64) -> [f64; 2] {
        let mut left = distance.max(0.0);
        let points = edge.points();
        for pair in points.windows(2) {
            let (a, b) = (self.local(pair[0]), self.local(pair[1]));
            let step = (b[0] - a[0]).hypot(b[1] - a[1]);
            if left <= step && step > 0.0 {
                let t = left / step;
                return [t.mul_add(b[0] - a[0], a[0]), t.mul_add(b[1] - a[1], a[1])];
            }
            left -= step;
        }
        points.last().map_or([0.0, 0.0], |&last| self.local(last))
    }
}

/// Per position in `xy` (metres), the indices of the others within
/// `radius` metres, ascending.
pub(crate) fn close_points(xy: &[[f64; 2]], radius: f64) -> Vec<Vec<u32>> {
    let cell = |p: [f64; 2]| {
        // SAFETY: airport coordinates are a few kilometres from the origin.
        #[allow(clippy::cast_possible_truncation)]
        let key = (
            (p[0] / radius).floor() as i64,
            (p[1] / radius).floor() as i64,
        );
        key
    };
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (index, &p) in xy.iter().enumerate() {
        grid.entry(cell(p))
            .or_default()
            .push(u32::try_from(index).unwrap_or(u32::MAX));
    }
    xy.iter()
        .enumerate()
        .map(|(index, &p)| {
            let (x, y) = cell(p);
            let mut near = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for &other in grid.get(&(x + dx, y + dy)).map_or(&[][..], Vec::as_slice) {
                        let q = xy[other as usize];
                        if other as usize != index && (q[0] - p[0]).hypot(q[1] - p[1]) <= radius {
                            near.push(other);
                        }
                    }
                }
            }
            near.sort_unstable();
            near
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::close_points;
    use crate::{ground::GroundNetwork, test_data::data};

    #[test]
    fn test_close_points_finds_neighbours_across_cells() {
        let xy = [
            [0.0, 0.0],
            [44.0, 0.0],
            [46.0, 0.0],
            [-30.0, -30.0],
            [500.0, 500.0],
        ];
        let near = close_points(&xy, 45.0);
        assert_eq!(near[0], [1, 3]);
        assert_eq!(near[1], [0, 2]);
        assert_eq!(near[2], [1]);
        assert_eq!(near[3], [0]);
        assert_eq!(near[4], [0_u32; 0]);
    }

    #[test]
    fn test_edge_point_walks_the_polyline() -> Result<()> {
        let world = data()?.world;
        let network = GroundNetwork::from_json(
            include_str!("../../../../assets/ground/germany.ground.json"),
            &world,
        )?;
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let ground = network.airport(frankfurt).context("layout")?;
        let edge = ground
            .edges()
            .iter()
            .find(|e| e.points().len() > 2)
            .context("a bent edge")?;
        let start = ground.edge_point(edge, 0.0);
        let from = ground.nodes()[edge.from().as_usize()].xy;
        assert!((start[0] - from[0]).hypot(start[1] - from[1]) < 0.01);
        let end = ground.edge_point(edge, edge.length().value());
        let to = ground.nodes()[edge.to().as_usize()].xy;
        assert!((end[0] - to[0]).hypot(end[1] - to[1]) < 0.5);
        let half = ground.edge_point(edge, edge.length().value() / 2.0);
        let chord = (to[0] - from[0]).hypot(to[1] - from[1]);
        assert!((half[0] - from[0]).hypot(half[1] - from[1]) <= chord);
        Ok(())
    }

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
