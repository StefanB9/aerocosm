//! Pure geometry and level of detail for drawing airport grounds (plan M4
//! D87): zoom thresholds, culling, stand markers and where taxiway names go.

use std::collections::BTreeMap;

use aerocosm_core::ground::GroundName;
use bevy::math::{Rect, Vec2};

/// Below this scale taxiway names are candidates for labels.
const TAXIWAY_NAME_SCALE: f32 = 6.0;
/// Below this scale stand names are candidates for labels.
const STAND_NAME_SCALE: f32 = 1.8;

/// Whether taxiway names are labelled at `scale`.
pub(crate) fn taxiway_names_visible(scale: f32) -> bool {
    scale < TAXIWAY_NAME_SCALE
}

/// Whether stand names are labelled at `scale`.
pub(crate) fn stand_names_visible(scale: f32) -> bool {
    scale < STAND_NAME_SCALE
}

/// The smallest rectangle around `points`, if there are any.
pub(crate) fn bounds(points: impl IntoIterator<Item = Vec2>) -> Option<Rect> {
    let mut points = points.into_iter();
    let first = points.next()?;
    Some(points.fold(Rect::from_corners(first, first), |rect, p| {
        rect.union_point(p)
    }))
}

/// Whether something within `bounds` shows in the visible `view`.
pub(crate) fn in_view(bounds: Rect, view: Rect) -> bool {
    !bounds.intersect(view).is_empty() || view.contains(bounds.min) || bounds.contains(view.min)
}

/// The direction of a true `heading` (degrees clockwise from north) on the
/// map, y up.
fn direction(heading: f32) -> Vec2 {
    let radians = heading.to_radians();
    Vec2::new(radians.sin(), radians.cos())
}

/// A stand marker at `at` for an aircraft parked nose towards `heading`,
/// `size` map units across: the stop bar across the nose and the lead line
/// behind it, as two segments.
pub(crate) fn stand_marker(at: Vec2, heading: f32, size: f32) -> [[Vec2; 2]; 2] {
    let ahead = direction(heading);
    let across = ahead.perp() * (size / 2.0);
    [[at - across, at + across], [at, at - ahead * size]]
}

/// The point halfway along the polyline through `points`.
pub(crate) fn polyline_middle(points: &[Vec2]) -> Option<Vec2> {
    let length: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = length / 2.0;
    for pair in points.windows(2) {
        let step = pair[0].distance(pair[1]);
        if left <= step && step > 0.0 {
            return Some(pair[0].lerp(pair[1], left / step));
        }
        left -= step;
    }
    points.first().copied()
}

/// A named taxiway edge, for label anchors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NamedEdge {
    pub(crate) name: GroundName,
    pub(crate) from: u32,
    pub(crate) to: u32,
    pub(crate) length: f32,
    /// Where the edge is halfway along it, in map units.
    pub(crate) middle: Vec2,
}

/// The root of `node`'s set in the union-find `parent`, compressing the
/// path to it.
fn root(parent: &mut BTreeMap<u32, u32>, node: u32) -> u32 {
    let mut current = node;
    while let Some(&up) = parent.get(&current)
        && up != current
    {
        current = up;
    }
    parent.insert(node, current);
    current
}

/// One label anchor per stretch of connected edges sharing a name: the
/// middle of its longest edge. Ordered by name, then the longest first.
pub(crate) fn stretch_anchors(edges: &[NamedEdge]) -> Vec<(GroundName, Vec2)> {
    let mut by_name: BTreeMap<GroundName, Vec<&NamedEdge>> = BTreeMap::new();
    for edge in edges {
        by_name.entry(edge.name).or_default().push(edge);
    }
    let mut anchors = Vec::with_capacity(by_name.len());
    for (name, group) in by_name {
        // Union-find over this name's nodes.
        let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
        for edge in &group {
            parent.entry(edge.from).or_insert(edge.from);
            parent.entry(edge.to).or_insert(edge.to);
            let (a, b) = (root(&mut parent, edge.from), root(&mut parent, edge.to));
            if a != b {
                parent.insert(a.max(b), a.min(b));
            }
        }
        let mut longest: BTreeMap<u32, &NamedEdge> = BTreeMap::new();
        for edge in &group {
            let stretch = root(&mut parent, edge.from);
            let best = longest.entry(stretch).or_insert(edge);
            if edge.length > best.length {
                *best = edge;
            }
        }
        let mut stretches: Vec<&NamedEdge> = longest.into_values().collect();
        stretches.sort_by(|a, b| b.length.total_cmp(&a.length));
        anchors.extend(stretches.into_iter().map(|edge| (name, edge.middle)));
    }
    anchors
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_ground_detail_appears_in_steps_when_zooming_in() {
        assert!(!taxiway_names_visible(10.0));
        assert!(taxiway_names_visible(3.0) && !stand_names_visible(3.0));
        assert!(stand_names_visible(1.0));
    }

    #[test]
    fn test_bounds_and_view_culling() -> Result<()> {
        assert!(bounds(std::iter::empty()).is_none());
        let rect = bounds([
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, -2.0),
            Vec2::new(1.0, 3.0),
        ])
        .context("bounds")?;
        assert_eq!(rect, Rect::new(0.0, -2.0, 4.0, 3.0));
        assert!(in_view(rect, Rect::new(3.0, 2.0, 10.0, 10.0)));
        assert!(!in_view(rect, Rect::new(5.0, 5.0, 10.0, 10.0)));
        // One inside the other.
        assert!(in_view(rect, Rect::new(1.0, 0.0, 2.0, 1.0)));
        assert!(in_view(Rect::new(1.0, 0.0, 1.0, 0.0), rect));
        Ok(())
    }

    #[test]
    fn test_stand_marker_bars_the_nose_and_leads_in_from_behind() {
        let [bar, lead] = stand_marker(Vec2::new(10.0, 10.0), 90.0, 4.0);
        // Facing east: the bar runs north-south, the lead line comes from the
        // west.
        assert!((bar[0] - Vec2::new(10.0, 8.0)).length() < 1e-4);
        assert!((bar[1] - Vec2::new(10.0, 12.0)).length() < 1e-4);
        assert!((lead[1] - Vec2::new(6.0, 10.0)).length() < 1e-4);
    }

    #[test]
    fn test_polyline_middle_walks_by_length() -> Result<()> {
        let points = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 30.0)];
        let middle = polyline_middle(&points).context("a middle")?;
        assert!((middle - Vec2::new(10.0, 10.0)).length() < 1e-4);
        assert_eq!(polyline_middle(&[Vec2::ONE]), Some(Vec2::ONE));
        assert_eq!(polyline_middle(&[]), None);
        Ok(())
    }

    #[test]
    fn test_stretch_anchors_label_each_connected_stretch_once() -> Result<()> {
        let (n, l): (GroundName, GroundName) = ("N".parse()?, "L".parse()?);
        let edge = |name, from, to, length, x| NamedEdge {
            name,
            from,
            to,
            length,
            middle: Vec2::new(x, 0.0),
        };
        let edges = [
            // N: 0-1-2 joined, and 7-8 apart.
            edge(n, 0, 1, 50.0, 1.0),
            edge(n, 2, 1, 80.0, 2.0),
            edge(n, 7, 8, 30.0, 3.0),
            // L: one edge, joined to N's nodes but a different name.
            edge(l, 1, 5, 20.0, 4.0),
        ];
        let anchors = stretch_anchors(&edges);
        assert_eq!(
            anchors,
            [
                (l, Vec2::new(4.0, 0.0)),
                (n, Vec2::new(2.0, 0.0)),
                (n, Vec2::new(3.0, 0.0)),
            ]
        );
        assert_eq!(stretch_anchors(&[]), []);
        Ok(())
    }
}
