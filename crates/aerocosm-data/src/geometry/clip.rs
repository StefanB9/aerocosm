//! Clipping rings and lines to a rectangle.

use super::{Point, Rect};

/// One side of the clip rectangle, as a half-plane test plus an edge
/// intersection.
#[derive(Clone, Copy)]
enum Edge {
    West(f64),
    East(f64),
    South(f64),
    North(f64),
}

impl Edge {
    fn all(rect: &Rect) -> [Self; 4] {
        [
            Self::West(rect.west),
            Self::East(rect.east),
            Self::South(rect.south),
            Self::North(rect.north),
        ]
    }

    fn inside(self, [lon, lat]: Point) -> bool {
        match self {
            Self::West(west) => lon >= west,
            Self::East(east) => lon <= east,
            Self::South(south) => lat >= south,
            Self::North(north) => lat <= north,
        }
    }

    /// Where segment `a`–`b` crosses this edge; the crossing coordinate is set
    /// exactly.
    fn intersect(self, a: Point, b: Point) -> Point {
        match self {
            Self::West(x) | Self::East(x) => [x, a[1] + (x - a[0]) / (b[0] - a[0]) * (b[1] - a[1])],
            Self::South(y) | Self::North(y) => {
                [a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]), y]
            }
        }
    }
}

/// Clips a closed ring to `rect` (Sutherland–Hodgman). Returns a closed ring,
/// or an empty vector when nothing remains. Concave rings may gain zero-area
/// slivers along the edges, which is harmless for filled rendering.
pub(crate) fn clip_ring(ring: &[Point], rect: &Rect) -> Vec<Point> {
    let open = match ring {
        [first, .., last] if first == last => &ring[..ring.len() - 1],
        _ => ring,
    };
    let mut current = open.to_vec();
    let mut next = Vec::with_capacity(current.len() + 4);
    for edge in Edge::all(rect) {
        next.clear();
        let Some(&last) = current.last() else {
            return Vec::new();
        };
        let mut previous = last;
        for &point in &current {
            match (edge.inside(previous), edge.inside(point)) {
                (true, true) => next.push(point),
                (false, true) => {
                    next.push(edge.intersect(previous, point));
                    next.push(point);
                }
                (true, false) => next.push(edge.intersect(previous, point)),
                (false, false) => {}
            }
            previous = point;
        }
        std::mem::swap(&mut current, &mut next);
    }
    current.dedup();
    if current.len() < 3 {
        return Vec::new();
    }
    current.push(current[0]);
    current
}

/// Clips a polyline to `rect` (Liang–Barsky per segment), splitting it where it
/// leaves and re-enters. Every returned piece has at least two points.
pub(crate) fn clip_line(line: &[Point], rect: &Rect) -> Vec<Vec<Point>> {
    let mut pieces: Vec<Vec<Point>> = Vec::new();
    let mut open = false;
    for segment in line.windows(2) {
        let Some((start, end)) = clip_segment(segment[0], segment[1], rect) else {
            open = false;
            continue;
        };
        match pieces.last_mut() {
            Some(piece) if open && piece.last() == Some(&start) => piece.push(end),
            _ => pieces.push(vec![start, end]),
        }
        // The piece continues only if this segment ended inside, unclipped.
        open = end == segment[1];
    }
    pieces
}

/// The part of segment `a`–`b` inside `rect`, if any. Unclipped ends are
/// returned exactly.
fn clip_segment(a: Point, b: Point, rect: &Rect) -> Option<(Point, Point)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    for (p, q) in [
        (-dx, a[0] - rect.west),
        (dx, rect.east - a[0]),
        (-dy, a[1] - rect.south),
        (dy, rect.north - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f64| {
        if t <= 0.0 {
            a
        } else if t >= 1.0 {
            b
        } else {
            [a[0] + t * dx, a[1] + t * dy]
        }
    };
    Some((at(t0), at(t1)))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::geometry::tests::GERMANY;

    fn closed(points: &[Point]) -> bool {
        points.first() == points.last()
    }

    #[test]
    fn test_clip_ring_inside_is_unchanged() {
        let ring = [[5.0, 50.0], [6.0, 50.0], [6.0, 51.0], [5.0, 50.0]];
        assert_eq!(clip_ring(&ring, &GERMANY), ring.to_vec());
    }

    #[test]
    fn test_clip_ring_outside_is_empty() {
        let ring = [[20.0, 50.0], [21.0, 50.0], [21.0, 51.0], [20.0, 50.0]];
        assert_eq!(clip_ring(&ring, &GERMANY).len(), 0);
    }

    #[test]
    fn test_clip_ring_cuts_at_edge() {
        // A square straddling the western edge at lon 3.
        let ring = [
            [2.0, 50.0],
            [4.0, 50.0],
            [4.0, 52.0],
            [2.0, 52.0],
            [2.0, 50.0],
        ];
        let clipped = clip_ring(&ring, &GERMANY);
        assert!(closed(&clipped));
        assert!(clipped.iter().all(|p| GERMANY.contains(*p)));
        assert!(clipped.iter().any(|p| (p[0] - 3.0).abs() < 1e-12));
        assert_eq!(clipped.len(), 5);
    }

    #[test]
    fn test_clip_line_inside_is_one_piece() {
        let line = [[5.0, 50.0], [6.0, 51.0], [7.0, 50.5]];
        assert_eq!(clip_line(&line, &GERMANY), vec![line.to_vec()]);
    }

    #[test]
    fn test_clip_line_leaving_and_reentering_splits() {
        let line = [[16.0, 50.0], [19.0, 50.0], [19.0, 51.0], [16.0, 51.0]];
        let pieces = clip_line(&line, &GERMANY);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0], vec![[16.0, 50.0], [17.5, 50.0]]);
        assert_eq!(pieces[1], vec![[17.5, 51.0], [16.0, 51.0]]);
    }

    #[test]
    fn test_clip_line_crossing_entirely() {
        let line = [[0.0, 50.0], [20.0, 50.0]];
        assert_eq!(
            clip_line(&line, &GERMANY),
            vec![vec![[3.0, 50.0], [17.5, 50.0]]]
        );
    }

    fn point() -> impl Strategy<Value = Point> {
        (-5.0_f64..25.0, 40.0_f64..62.0).prop_map(|(lon, lat)| [lon, lat])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_clip_ring_output_inside_and_closed(mut ring in prop::collection::vec(point(), 3..30)) {
            ring.push(ring[0]);
            let clipped = clip_ring(&ring, &GERMANY);
            prop_assert!(clipped.is_empty() || (clipped.len() >= 4 && closed(&clipped)));
            prop_assert!(clipped.iter().all(|p| GERMANY.contains(*p)));
        }

        #[test]
        fn test_clip_line_pieces_inside_with_two_points(line in prop::collection::vec(point(), 2..30)) {
            for piece in clip_line(&line, &GERMANY) {
                prop_assert!(piece.len() >= 2);
                prop_assert!(piece.iter().all(|p| GERMANY.contains(*p)));
            }
        }

        #[test]
        fn test_clip_line_keeps_inside_vertices(line in prop::collection::vec(point(), 2..30)) {
            let pieces = clip_line(&line, &GERMANY);
            for vertex in line.iter().filter(|p| GERMANY.contains(**p)) {
                // A lone inside vertex whose neighbours are both outside still lies on a
                // clipped piece, as a segment end.
                prop_assert!(pieces.iter().flatten().any(|p| p == vertex));
            }
        }
    }
}
