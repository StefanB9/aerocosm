//! A flat frame in metres around an airport. Over a few kilometres the error
//! of an equirectangular projection is far below OSM's mapping accuracy.

/// East and north of the frame's origin, in metres.
pub(crate) type Xy = [f64; 2];

/// Metres per degree of latitude (mean).
const METRES_PER_DEGREE: f64 = 111_195.0;

/// An equirectangular projection around an origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plane {
    lat0: f64,
    lon0: f64,
    /// Metres per degree of longitude at the origin.
    kx: f64,
}

impl Plane {
    /// A frame around `[lat, lon]`.
    pub(crate) fn new([lat0, lon0]: [f64; 2]) -> Self {
        Self {
            lat0,
            lon0,
            kx: METRES_PER_DEGREE * lat0.to_radians().cos(),
        }
    }

    /// `[lat, lon]` to metres.
    pub(crate) fn project(&self, [lat, lon]: [f64; 2]) -> Xy {
        [
            (lon - self.lon0) * self.kx,
            (lat - self.lat0) * METRES_PER_DEGREE,
        ]
    }

    /// Metres to `[lat, lon]`, rounded to 7 decimals (about 1 cm) so output
    /// is stable.
    pub(crate) fn unproject(&self, [x, y]: Xy) -> [f64; 2] {
        [
            round7(self.lat0 + y / METRES_PER_DEGREE),
            round7(self.lon0 + x / self.kx),
        ]
    }
}

fn round7(value: f64) -> f64 {
    (value * 1e7).round() / 1e7
}

/// Distance between two points.
pub(crate) fn distance(a: Xy, b: Xy) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// True bearing from `a` to `b`, `[0, 360)`.
pub(crate) fn bearing(a: Xy, b: Xy) -> f64 {
    (b[0] - a[0])
        .atan2(b[1] - a[1])
        .to_degrees()
        .rem_euclid(360.0)
}

/// Smallest angle between two bearings, `[0, 180]`.
pub(crate) fn angle_between(a: f64, b: f64) -> f64 {
    let difference = (a - b).rem_euclid(360.0);
    difference.min(360.0 - difference)
}

/// The point of segment `a`–`b` closest to `p`, and its fraction along the
/// segment.
pub(crate) fn closest_on_segment(point: Xy, start: Xy, end: Xy) -> (Xy, f64) {
    let along = [end[0] - start[0], end[1] - start[1]];
    let length2 = along[0] * along[0] + along[1] * along[1];
    if length2 <= f64::EPSILON {
        return (start, 0.0);
    }
    let fraction = (((point[0] - start[0]) * along[0] + (point[1] - start[1]) * along[1])
        / length2)
        .clamp(0.0, 1.0);
    (
        [
            start[0] + fraction * along[0],
            start[1] + fraction * along[1],
        ],
        fraction,
    )
}

/// Whether `p` lies inside the closed `ring` (even-odd rule).
pub(crate) fn inside(p: Xy, ring: &[Xy]) -> bool {
    let mut result = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            result = !result;
        }
        j = i;
    }
    result
}

/// Distance from `p` to the nearest point of the polyline `line`.
pub(crate) fn distance_to_line(p: Xy, line: &[Xy]) -> f64 {
    match line {
        [] => f64::INFINITY,
        [only] => distance(p, *only),
        _ => line
            .windows(2)
            .map(|pair| distance(p, closest_on_segment(p, pair[0], pair[1]).0))
            .fold(f64::INFINITY, f64::min),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn test_plane_projects_metres_around_the_origin() {
        let plane = Plane::new([50.0, 8.5]);
        let east = plane.project([50.0, 8.51]);
        assert!(close(east[0], 714.7, 0.5), "{east:?}");
        assert!(close(east[1], 0.0, 1e-9));
        let north = plane.project([50.01, 8.5]);
        assert!(close(north[1], 1_111.95, 0.01), "{north:?}");
        assert_eq!(plane.unproject([0.0, 0.0]), [50.0, 8.5]);
    }

    #[test]
    fn test_bearings_and_angles() {
        assert!(close(bearing([0.0, 0.0], [0.0, 10.0]), 0.0, 1e-9));
        assert!(close(bearing([0.0, 0.0], [10.0, 0.0]), 90.0, 1e-9));
        assert!(close(bearing([0.0, 0.0], [-10.0, 0.0]), 270.0, 1e-9));
        assert!(close(angle_between(350.0, 10.0), 20.0, 1e-9));
        assert!(close(angle_between(90.0, 270.0), 180.0, 1e-9));
    }

    #[test]
    fn test_closest_point_inside_and_line_distance() {
        let (point, t) = closest_on_segment([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]);
        assert_eq!(point, [5.0, 0.0]);
        assert!(close(t, 0.5, 1e-12));
        let (point, t) = closest_on_segment([-4.0, 1.0], [0.0, 0.0], [10.0, 0.0]);
        assert_eq!((point, t), ([0.0, 0.0], 0.0));
        assert_eq!(
            closest_on_segment([1.0, 1.0], [2.0, 2.0], [2.0, 2.0]).0,
            [2.0, 2.0]
        );
        let square = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]];
        assert!(inside([2.0, 2.0], &square));
        assert!(!inside([5.0, 2.0], &square));
        assert!(close(distance_to_line([2.0, 7.0], &square), 3.0, 1e-12));
        assert!(distance_to_line([0.0, 0.0], &[]).is_infinite());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_plane_round_trips_within_a_centimetre(
            lat in 47.0_f64..55.0, lon in 6.0_f64..15.0,
            dlat in -0.05_f64..0.05, dlon in -0.08_f64..0.08,
        ) {
            let plane = Plane::new([lat, lon]);
            let point = [lat + dlat, lon + dlon];
            let back = plane.unproject(plane.project(point));
            prop_assert!(close(back[0], point[0], 1e-7) && close(back[1], point[1], 1e-7));
        }

        #[test]
        fn test_closest_point_is_no_farther_than_the_ends(
            px in -100.0_f64..100.0, py in -100.0_f64..100.0,
            ax in -100.0_f64..100.0, ay in -100.0_f64..100.0,
            bx in -100.0_f64..100.0, by in -100.0_f64..100.0,
        ) {
            let p = [px, py];
            let (q, t) = closest_on_segment(p, [ax, ay], [bx, by]);
            prop_assert!((0.0..=1.0).contains(&t));
            prop_assert!(distance(p, q) <= distance(p, [ax, ay]) + 1e-9);
            prop_assert!(distance(p, q) <= distance(p, [bx, by]) + 1e-9);
        }
    }
}
