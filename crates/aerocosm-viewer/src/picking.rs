//! Hit-testing map symbols under the pointer.

use bevy::math::Vec2;

/// The symbol closest to `cursor` within `reach` pixels, if any.
pub(crate) fn nearest<T>(
    symbols: impl IntoIterator<Item = (T, Vec2)>,
    cursor: Vec2,
    reach: f32,
) -> Option<T> {
    symbols
        .into_iter()
        .map(|(item, position)| (item, position.distance_squared(cursor)))
        .filter(|(_, distance)| *distance <= reach * reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(item, _)| item)
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_nearest_within_reach() {
        let symbols = [
            (1_u32, Vec2::new(100.0, 100.0)),
            (2, Vec2::new(104.0, 100.0)),
            (3, Vec2::new(300.0, 300.0)),
        ];
        assert_eq!(nearest(symbols, Vec2::new(103.0, 100.0), 10.0), Some(2));
        assert_eq!(nearest(symbols, Vec2::new(200.0, 200.0), 10.0), None);
        assert_eq!(
            nearest(std::iter::empty::<(u32, Vec2)>(), Vec2::ZERO, 10.0),
            None
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_nearest_is_closest_and_within_reach(
            points in prop::collection::vec((0.0_f32..500.0, 0.0_f32..500.0), 0..40),
            cx in 0.0_f32..500.0, cy in 0.0_f32..500.0, reach in 1.0_f32..50.0,
        ) {
            let cursor = Vec2::new(cx, cy);
            let symbols: Vec<(usize, Vec2)> = points.iter().enumerate().map(|(i, (x, y))| (i, Vec2::new(*x, *y))).collect();
            let found = nearest(symbols.iter().copied(), cursor, reach);
            let closest = symbols.iter().map(|(_, p)| p.distance(cursor)).fold(f32::INFINITY, f32::min);
            match found {
                Some(index) => {
                    let distance = symbols[index].1.distance(cursor);
                    prop_assert!(distance <= reach);
                    prop_assert!((distance - closest).abs() < 1e-4);
                }
                None => prop_assert!(closest > reach),
            }
        }
    }
}
