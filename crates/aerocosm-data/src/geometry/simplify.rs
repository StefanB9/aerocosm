//! Douglas–Peucker polyline simplification.

use super::Point;

/// Drops points that lie within `tolerance` of the simplified line, keeping
/// both endpoints. Iterative, so long coastlines cannot overflow the stack.
pub(crate) fn simplify(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = Vec::with_capacity(64);
    stack.push((0, points.len() - 1));
    while let Some((first, last)) = stack.pop() {
        let mut farthest = first;
        let mut max_distance = 0.0;
        for index in first + 1..last {
            let distance = segment_distance(points[index], points[first], points[last]);
            if distance > max_distance {
                max_distance = distance;
                farthest = index;
            }
        }
        if max_distance > tolerance {
            keep[farthest] = true;
            stack.push((first, farthest));
            stack.push((farthest, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(point, kept)| kept.then_some(*point))
        .collect()
}

/// Distance from `p` to the segment `a`–`b` (not the infinite line), so every
/// dropped point is within tolerance of the output polyline itself.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length_squared = dx * dx + dy * dy;
        let t = if length_squared == 0.0 {
            0.0
        } else {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
        };
        ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
    }

    #[test]
    fn test_simplify_drops_collinear_points() {
        let line = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        assert_eq!(simplify(&line, 0.01), vec![[0.0, 0.0], [3.0, 0.0]]);
    }

    #[test]
    fn test_simplify_keeps_significant_corner() {
        let line = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]];
        assert_eq!(simplify(&line, 0.5), line.to_vec());
        assert_eq!(simplify(&line, 2.0), vec![[0.0, 0.0], [2.0, 0.0]]);
    }

    #[test]
    fn test_simplify_short_inputs_unchanged() {
        assert_eq!(simplify(&[], 1.0), Vec::<Point>::new());
        assert_eq!(simplify(&[[1.0, 2.0]], 1.0), vec![[1.0, 2.0]]);
        assert_eq!(
            simplify(&[[1.0, 2.0], [3.0, 4.0]], 1.0),
            vec![[1.0, 2.0], [3.0, 4.0]]
        );
    }

    #[test]
    fn test_simplify_closed_ring_keeps_endpoints() {
        let ring = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let simplified = simplify(&ring, 0.1);
        assert_eq!(simplified.first(), simplified.last());
        assert_eq!(simplified.len(), 5);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_simplify_is_ordered_subset_with_endpoints(
            line in prop::collection::vec((-10.0_f64..10.0, -10.0_f64..10.0).prop_map(|(x, y)| [x, y]), 2..60),
            tolerance in 0.0_f64..2.0,
        ) {
            let simplified = simplify(&line, tolerance);
            prop_assert_eq!(simplified.first(), line.first());
            prop_assert_eq!(simplified.last(), line.last());
            let mut cursor = line.iter();
            for kept in &simplified {
                prop_assert!(cursor.any(|p| p == kept), "kept points appear in input order");
            }
        }

        #[test]
        fn test_simplify_stays_within_tolerance(
            line in prop::collection::vec((-10.0_f64..10.0, -10.0_f64..10.0).prop_map(|(x, y)| [x, y]), 2..60),
            tolerance in 0.0_f64..2.0,
        ) {
            let simplified = simplify(&line, tolerance);
            for point in &line {
                let nearest = simplified
                    .windows(2)
                    .map(|w| segment_distance(*point, w[0], w[1]))
                    .fold(f64::INFINITY, f64::min);
                prop_assert!(nearest <= tolerance + 1e-9, "{point:?} is {nearest} away");
            }
        }
    }
}
