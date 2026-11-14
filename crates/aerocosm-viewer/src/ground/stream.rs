//! Which airports' grounds are built (plan M5 D132): pure, so that the
//! streaming rules are tested apart from Bevy. An airport is built when it
//! comes into view at airport zoom and dropped once it is well out of view
//! or the camera is far out; in between it stays built, so panning back and
//! forth or zooming a little does not rebuild it.

use bevy::math::Rect;

use super::geometry::in_view;

/// Below this scale (map units per pixel) grounds in view are built: an
/// airport like Frankfurt spans a few hundred pixels.
const BUILD_SCALE: f32 = 30.0;
/// Above this scale every built ground is dropped.
const DROP_SCALE: f32 = 120.0;
/// Grounds this many view sizes around the view are built…
const BUILD_MARGIN: f32 = 0.25;
/// …and those beyond this many are dropped.
const DROP_MARGIN: f32 = 1.5;
/// At most this many grounds are built per frame, so a hub's tessellation and
/// labels do not land in one frame together with others.
pub(crate) const BUILDS_PER_FRAME: usize = 2;

/// What to change this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StreamPlan {
    /// Airports to build, nearest the view centre first.
    pub(crate) build: Vec<usize>,
    /// Built airports to drop.
    pub(crate) drop: Vec<usize>,
}

/// The plan for airports with extents `areas` (index, extent in map units)
/// when the camera shows `view` at `scale`; `built` tells whether an airport
/// is built.
pub(crate) fn plan(
    areas: impl IntoIterator<Item = (usize, Rect)>,
    view: Rect,
    scale: f32,
    built: impl Fn(usize) -> bool,
) -> StreamPlan {
    let grow = |margin: f32| {
        let extra = view.size() * margin;
        Rect::from_corners(view.min - extra, view.max + extra)
    };
    let (near, far) = (grow(BUILD_MARGIN), grow(DROP_MARGIN));
    let mut plan = StreamPlan::default();
    let mut wanted: Vec<(f32, usize)> = Vec::new();
    for (index, area) in areas {
        let is_built = built(index);
        if is_built && (scale > DROP_SCALE || !in_view(area, far)) {
            plan.drop.push(index);
        } else if !is_built && scale < BUILD_SCALE && in_view(area, near) {
            wanted.push((area.center().distance_squared(view.center()), index));
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    plan.build = wanted
        .into_iter()
        .take(BUILDS_PER_FRAME)
        .map(|(_, index)| index)
        .collect();
    plan
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use proptest::prelude::*;

    use super::*;

    fn square(x: f32, y: f32, size: f32) -> Rect {
        Rect::from_center_size(Vec2::new(x, y), Vec2::splat(size))
    }

    const VIEW: Rect = Rect {
        min: Vec2::new(-500.0, -300.0),
        max: Vec2::new(500.0, 300.0),
    };

    #[test]
    fn test_airports_in_view_are_built_nearest_first() {
        let areas = [
            (0, square(400.0, 0.0, 100.0)),
            (1, square(0.0, 0.0, 100.0)),
            (2, square(200.0, 0.0, 100.0)),
            (3, square(5_000.0, 0.0, 100.0)),
        ];
        let plan = plan(areas, VIEW, 10.0, |_| false);
        assert_eq!(plan.build, [1, 2], "two per frame, nearest first");
        assert_eq!(plan.drop, Vec::<usize>::new());
    }

    #[test]
    fn test_nothing_is_built_far_out() {
        let plan = plan([(0, square(0.0, 0.0, 100.0))], VIEW, 60.0, |_| false);
        assert_eq!(plan, StreamPlan::default());
    }

    #[test]
    fn test_built_grounds_stay_until_well_out_of_view_or_far_out() {
        // Just outside the view: kept.
        let beside = [(0, square(900.0, 0.0, 100.0))];
        assert_eq!(plan(beside, VIEW, 10.0, |_| true), StreamPlan::default());
        // Zoomed out a little: kept.
        assert_eq!(plan(beside, VIEW, 60.0, |_| true), StreamPlan::default());
        // Far out, or far away: dropped.
        assert_eq!(plan(beside, VIEW, 200.0, |_| true).drop, [0]);
        let away = [(0, square(5_000.0, 0.0, 100.0))];
        assert_eq!(plan(away, VIEW, 10.0, |_| true).drop, [0]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// An airport is never both built and dropped, built ones are never
        /// built again, and at most the per-frame budget is built.
        #[test]
        fn test_plans_are_consistent(
            airports in proptest::collection::vec((-5e3_f32..5e3, -5e3_f32..5e3, 1.0_f32..2e3, any::<bool>()), 0..40),
            scale in 0.5_f32..500.0,
        ) {
            let areas: Vec<(usize, Rect)> = airports
                .iter()
                .enumerate()
                .map(|(i, (x, y, size, _))| (i, square(*x, *y, *size)))
                .collect();
            let built = |i: usize| airports[i].3;
            let plan = plan(areas.iter().copied(), VIEW, scale, built);
            prop_assert!(plan.build.len() <= BUILDS_PER_FRAME);
            for index in &plan.build {
                prop_assert!(!built(*index));
                prop_assert!(!plan.drop.contains(index));
            }
            for index in &plan.drop {
                prop_assert!(built(*index));
            }
            // What is built and in view is never dropped.
            for (index, area) in &areas {
                if built(*index) && scale <= DROP_SCALE && in_view(*area, VIEW) {
                    prop_assert!(!plan.drop.contains(index));
                }
            }
        }
    }
}
