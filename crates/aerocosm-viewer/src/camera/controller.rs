//! Camera maths: pure functions on [`CameraState`], unit- and property-tested
//! (plan D24).
//!
//! Screen coordinates follow Bevy's window convention (origin top-left, y down,
//! pixels); world coordinates are local map units (y up).

use bevy::math::{Rect, Vec2};

/// How fast a queued zoom is applied: the remaining amount decays at this rate
/// per second.
const ZOOM_EASE_RATE: f32 = 18.0;
/// How fast a released pan slows down, per second.
const FRICTION: f32 = 6.0;
/// Below this on-screen speed (pixels per second) coasting stops.
const STOP_SPEED: f32 = 5.0;
/// A queued zoom this close to done (natural log of the factor) is finished in
/// one go.
const ZOOM_SNAP: f32 = 1e-4;

/// Where the camera may go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CameraLimits {
    /// The camera centre stays inside this rectangle.
    pub(crate) bounds: Rect,
    /// Closest zoom, in world units per pixel.
    pub(crate) min_scale: f32,
    /// Farthest zoom, in world units per pixel.
    pub(crate) max_scale: f32,
    /// Widest view, in world units: the world's width where east and west
    /// wrap, so nothing is seen twice side by side (plan M5 D131).
    pub(crate) max_width: f32,
}

/// Position, zoom and motion of the 2D map camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CameraState {
    center: Vec2,
    /// World units per screen pixel.
    scale: f32,
    /// Coasting velocity in world units per second.
    velocity: Vec2,
    /// Natural log of the zoom factor still to apply.
    pending_zoom: f32,
    /// Screen point the pending zoom is anchored to.
    zoom_anchor: Vec2,
    /// Last known window size, used to keep the visible area inside the bounds.
    viewport: Vec2,
}

impl CameraState {
    /// A camera at rest.
    pub(crate) const fn new(center: Vec2, scale: f32) -> Self {
        Self {
            center,
            scale,
            velocity: Vec2::ZERO,
            pending_zoom: 0.0,
            zoom_anchor: Vec2::ZERO,
            viewport: Vec2::ZERO,
        }
    }

    /// A camera showing all of `bounds` in a `viewport`-sized window.
    pub(crate) fn fit(bounds: Rect, viewport: Vec2, limits: &CameraLimits) -> Self {
        let size = bounds.size();
        let scale = (size.x / viewport.x).max(size.y / viewport.y);
        let mut state = Self::new(bounds.center(), scale);
        state.viewport = viewport;
        state.clamp(limits);
        state
    }

    /// World point at the centre of the screen.
    pub(crate) const fn center(&self) -> Vec2 {
        self.center
    }

    /// World units per screen pixel.
    pub(crate) const fn scale(&self) -> f32 {
        self.scale
    }

    /// World position under a screen point.
    pub(crate) fn screen_to_world(&self, screen: Vec2, viewport: Vec2) -> Vec2 {
        let offset = screen - viewport / 2.0;
        self.center + Vec2::new(offset.x, -offset.y) * self.scale
    }

    /// Screen point showing a world position.
    pub(crate) fn world_to_screen(&self, world: Vec2, viewport: Vec2) -> Vec2 {
        let offset = (world - self.center) / self.scale;
        viewport / 2.0 + Vec2::new(offset.x, -offset.y)
    }

    /// Drags the map by a pointer movement, so content follows the pointer.
    pub(crate) fn pan_by_pixels(&mut self, delta: Vec2, limits: &CameraLimits) {
        self.center -= Vec2::new(delta.x, -delta.y) * self.scale;
        self.velocity = Vec2::ZERO;
        self.clamp(limits);
    }

    /// Ends a drag that was moving at `pointer_velocity` pixels per second; the
    /// map coasts.
    pub(crate) fn release(&mut self, pointer_velocity: Vec2) {
        self.velocity = Vec2::new(-pointer_velocity.x, pointer_velocity.y) * self.scale;
    }

    /// Zooms by `factor` (below 1 zooms in) keeping the world point under
    /// `cursor` fixed.
    pub(crate) fn zoom_about(
        &mut self,
        cursor: Vec2,
        factor: f32,
        viewport: Vec2,
        limits: &CameraLimits,
    ) {
        self.viewport = viewport;
        let anchor = self.screen_to_world(cursor, viewport);
        let scale = (self.scale * factor).clamp(limits.min_scale, self.max_scale(limits));
        self.center = anchor + (self.center - anchor) * (scale / self.scale);
        self.scale = scale;
        self.clamp(limits);
    }

    /// Adds a zoom of `ln_factor` (natural log) towards `anchor`, applied
    /// gradually by [`Self::step`] so wheel steps feel smooth.
    pub(crate) fn queue_zoom(&mut self, ln_factor: f32, anchor: Vec2) {
        self.pending_zoom += ln_factor;
        self.zoom_anchor = anchor;
    }

    /// Two-finger zoom: fingers moved from `previous_distance` to `distance`
    /// apart.
    pub(crate) fn pinch(
        &mut self,
        previous_distance: f32,
        distance: f32,
        midpoint: Vec2,
        viewport: Vec2,
        limits: &CameraLimits,
    ) {
        if previous_distance > 0.0 && distance > 0.0 {
            self.zoom_about(midpoint, previous_distance / distance, viewport, limits);
        }
    }

    /// Advances easing and coasting by `dt` seconds.
    pub(crate) fn step(&mut self, dt: f32, viewport: Vec2, limits: &CameraLimits) {
        self.viewport = viewport;
        if self.pending_zoom != 0.0 {
            let mut portion = self.pending_zoom * (1.0 - (-ZOOM_EASE_RATE * dt).exp());
            if (self.pending_zoom - portion).abs() < ZOOM_SNAP {
                portion = self.pending_zoom;
            }
            self.zoom_about(self.zoom_anchor, portion.exp(), viewport, limits);
            self.pending_zoom -= portion;
        }
        if self.velocity != Vec2::ZERO {
            self.center += self.velocity * dt;
            self.velocity *= (-FRICTION * dt).exp();
            if self.velocity.length() / self.scale < STOP_SPEED {
                self.velocity = Vec2::ZERO;
            }
            self.clamp(limits);
        }
    }

    /// Moves the centre by `-by` without touching zoom or motion: the
    /// floating origin moved to `by` (plan M5 D130).
    pub(crate) fn shift(&mut self, by: Vec2) {
        self.center -= by;
    }

    /// Centres on `target` at the current zoom and stops any coasting.
    pub(crate) fn follow(&mut self, target: Vec2, limits: &CameraLimits) {
        self.center = target;
        self.velocity = Vec2::ZERO;
        self.clamp(limits);
    }

    /// The farthest zoom in the current viewport.
    fn max_scale(&self, limits: &CameraLimits) -> f32 {
        let widest = if self.viewport.x > 0.0 {
            limits.max_width / self.viewport.x
        } else {
            f32::INFINITY
        };
        limits.max_scale.min(widest).max(limits.min_scale)
    }

    /// Pulls the scale within `limits` and moves the centre so the visible area
    /// stays inside the bounds; on an axis where the view is wider than the
    /// bounds, it is centred.
    pub(crate) fn clamp(&mut self, limits: &CameraLimits) {
        self.scale = self.scale.clamp(limits.min_scale, self.max_scale(limits));
        let half = self.viewport * self.scale / 2.0;
        let low = limits.bounds.min + half;
        let high = limits.bounds.max - half;
        let centre = limits.bounds.center();
        let axis = |value: f32, low: f32, high: f32, centre: f32| {
            if low > high {
                centre
            } else {
                value.clamp(low, high)
            }
        };
        self.center = Vec2::new(
            axis(self.center.x, low.x, high.x, centre.x),
            axis(self.center.y, low.y, high.y, centre.y),
        );
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{Rect, Vec2};
    use proptest::prelude::*;

    use super::*;

    const VIEWPORT: Vec2 = Vec2::new(1280.0, 800.0);

    fn limits() -> CameraLimits {
        CameraLimits {
            bounds: Rect::new(-700_000.0, -900_000.0, 700_000.0, 900_000.0),
            min_scale: 0.5,
            max_scale: 4000.0,
            max_width: f32::INFINITY,
        }
    }

    fn state(scale: f32) -> CameraState {
        CameraState::new(Vec2::new(10_000.0, -20_000.0), scale)
    }

    #[test]
    fn test_view_is_at_most_max_width_wide() {
        let limits = CameraLimits {
            max_width: 640_000.0,
            ..limits()
        };
        let mut camera = state(4000.0);
        camera.zoom_about(VIEWPORT / 2.0, 1.0, VIEWPORT, &limits);
        assert!((camera.scale() - 500.0).abs() < 1e-3, "{}", camera.scale());
    }

    #[test]
    fn test_unbounded_axis_pans_freely() {
        // East–west wraps (plan M5 D131): no bounds on x.
        let limits = CameraLimits {
            bounds: Rect::new(f32::NEG_INFINITY, -1000.0, f32::INFINITY, 1000.0),
            min_scale: 0.1,
            max_scale: 1.0,
            max_width: f32::INFINITY,
        };
        let mut camera = CameraState::new(Vec2::ZERO, 1.0);
        camera.pan_by_pixels(Vec2::new(-5e6, 0.0), &limits);
        assert!(
            (camera.center().x - 5e6).abs() < 1.0,
            "{:?}",
            camera.center()
        );
        assert!(camera.center().y.is_finite());
    }

    #[test]
    fn test_shift_moves_the_centre_only() {
        let mut camera = state(2.0);
        camera.release(Vec2::new(10.0, 0.0));
        camera.shift(Vec2::new(10_000.0, -20_000.0));
        assert_eq!(camera.center(), Vec2::ZERO);
        assert!((camera.scale() - 2.0).abs() < f32::EPSILON);
        camera.step(0.1, VIEWPORT, &limits());
        assert!(camera.center().x < 0.0, "still coasting");
    }

    #[test]
    fn test_follow_centres_and_stops_coasting() {
        let limits = CameraLimits {
            bounds: Rect::new(-1000.0, -1000.0, 1000.0, 1000.0),
            min_scale: 0.1,
            max_scale: 10.0,
            max_width: f32::INFINITY,
        };
        let mut state = CameraState::new(Vec2::ZERO, 1.0);
        state.release(Vec2::new(500.0, 0.0));
        state.follow(Vec2::new(100.0, 50.0), &limits);
        assert_eq!(state.center(), Vec2::new(100.0, 50.0));
        state.step(0.1, Vec2::new(200.0, 200.0), &limits);
        assert_eq!(
            state.center(),
            Vec2::new(100.0, 50.0),
            "no coasting after follow"
        );
    }

    #[test]
    fn test_screen_world_centre_and_axes() {
        let camera = state(10.0);
        assert_eq!(
            camera.screen_to_world(VIEWPORT / 2.0, VIEWPORT),
            camera.center()
        );
        let right_up = camera.screen_to_world(VIEWPORT / 2.0 + Vec2::new(10.0, -10.0), VIEWPORT);
        assert_eq!(
            right_up,
            camera.center() + Vec2::new(100.0, 100.0),
            "screen y down is world y up"
        );
    }

    #[test]
    fn test_fit_shows_whole_bounds() {
        let bounds = limits().bounds;
        let camera = CameraState::fit(bounds, VIEWPORT, &limits());
        let top_left = camera.screen_to_world(Vec2::ZERO, VIEWPORT);
        let bottom_right = camera.screen_to_world(VIEWPORT, VIEWPORT);
        assert!(top_left.x <= bounds.min.x + 1.0 && top_left.y >= bounds.max.y - 1.0);
        assert!(bottom_right.x >= bounds.max.x - 1.0 && bottom_right.y <= bounds.min.y + 1.0);
        assert_eq!(camera.center(), bounds.center());
    }

    #[test]
    fn test_pan_moves_content_with_the_pointer() {
        let mut camera = state(10.0);
        let grabbed = camera.screen_to_world(Vec2::new(300.0, 200.0), VIEWPORT);
        camera.pan_by_pixels(Vec2::new(50.0, -20.0), &limits());
        let now_under = camera.world_to_screen(grabbed, VIEWPORT);
        assert!(
            (now_under - Vec2::new(350.0, 180.0)).length() < 1e-3,
            "{now_under:?}"
        );
    }

    #[test]
    fn test_clamp_keeps_visible_area_inside_bounds() {
        let mut camera = state(100.0);
        camera.zoom_about(VIEWPORT / 2.0, 1.0, VIEWPORT, &limits());
        camera.pan_by_pixels(Vec2::new(100_000.0, 0.0), &limits());
        let left_edge = camera.screen_to_world(Vec2::ZERO, VIEWPORT).x;
        assert!(
            (left_edge - limits().bounds.min.x).abs() < 1.0,
            "left edge {left_edge}"
        );
    }

    #[test]
    fn test_clamp_centres_axis_wider_than_bounds() {
        let mut camera = state(3000.0);
        camera.zoom_about(VIEWPORT / 2.0, 1.0, VIEWPORT, &limits());
        assert_eq!(camera.center().x, limits().bounds.center().x);
    }

    #[test]
    fn test_clamp_limits_scale_and_centre() {
        let mut camera = CameraState::new(Vec2::new(5e6, -5e6), 1e6);
        camera.clamp(&limits());
        assert_eq!(camera.scale(), 4000.0);
        assert!(limits().bounds.contains(camera.center()));
        let mut close = state(0.01);
        close.clamp(&limits());
        assert_eq!(close.scale(), 0.5);
    }

    #[test]
    fn test_queued_zoom_is_eased_and_completes() {
        let mut camera = state(100.0);
        camera.queue_zoom(-2.0_f32.ln(), VIEWPORT / 2.0);
        camera.step(1.0 / 60.0, VIEWPORT, &limits());
        let after_one_frame = camera.scale();
        assert!(
            after_one_frame < 100.0 && after_one_frame > 50.0,
            "eased, not instant: {after_one_frame}"
        );
        for _ in 0..120 {
            camera.step(1.0 / 60.0, VIEWPORT, &limits());
        }
        assert!((camera.scale() - 50.0).abs() < 0.01, "{}", camera.scale());
    }

    #[test]
    fn test_inertia_coasts_and_stops() {
        let mut camera = state(10.0);
        let start = camera.center();
        camera.release(Vec2::new(-600.0, 0.0));
        camera.step(1.0 / 60.0, VIEWPORT, &limits());
        assert!(
            camera.center().x > start.x,
            "dragging left moves the view right"
        );
        for _ in 0..600 {
            camera.step(1.0 / 60.0, VIEWPORT, &limits());
        }
        assert_eq!(camera.velocity, Vec2::ZERO, "coasting has stopped");
    }

    #[test]
    fn test_pinch_zooms_about_midpoint() {
        let mut camera = state(10.0);
        let midpoint = Vec2::new(400.0, 300.0);
        let anchor = camera.screen_to_world(midpoint, VIEWPORT);
        camera.pinch(100.0, 200.0, midpoint, VIEWPORT, &limits());
        assert!((camera.scale() - 5.0).abs() < 1e-4);
        assert!((camera.world_to_screen(anchor, VIEWPORT) - midpoint).length() < 1e-2);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_screen_world_round_trip(x in 0.0_f32..1280.0, y in 0.0_f32..800.0, scale in 0.5_f32..4000.0) {
            let camera = state(scale);
            let screen = Vec2::new(x, y);
            let back = camera.world_to_screen(camera.screen_to_world(screen, VIEWPORT), VIEWPORT);
            prop_assert!((back - screen).length() < 0.05, "{back:?} vs {screen:?}");
        }

        #[test]
        fn test_zoom_about_keeps_anchor_under_cursor(
            x in 0.0_f32..1280.0, y in 0.0_f32..800.0, scale in 1.0_f32..1000.0, factor in 0.25_f32..4.0,
        ) {
            let mut camera = state(scale);
            let cursor = Vec2::new(x, y);
            let anchor = camera.screen_to_world(cursor, VIEWPORT);
            camera.zoom_about(cursor, factor, VIEWPORT, &CameraLimits { bounds: Rect::new(-1e7, -1e7, 1e7, 1e7), ..limits() });
            prop_assert!((camera.world_to_screen(anchor, VIEWPORT) - cursor).length() < 0.05);
        }

        #[test]
        fn test_clamp_always_within_limits(cx in -1e8_f32..1e8, cy in -1e8_f32..1e8, scale in 1e-3_f32..1e7) {
            let mut camera = CameraState::new(Vec2::new(cx, cy), scale);
            camera.zoom_about(VIEWPORT / 2.0, 1.0, VIEWPORT, &limits());
            prop_assert!((0.5..=4000.0).contains(&camera.scale()));
            prop_assert!(limits().bounds.contains(camera.center()));
        }

        #[test]
        fn test_visible_area_stays_inside_bounds_when_it_fits(
            cx in -1e7_f32..1e7, cy in -1e7_f32..1e7, scale in 0.5_f32..1000.0,
        ) {
            let mut camera = CameraState::new(Vec2::new(cx, cy), scale);
            camera.zoom_about(VIEWPORT / 2.0, 1.0, VIEWPORT, &limits());
            let bounds = limits().bounds.inflate(1.0);
            prop_assert!(bounds.contains(camera.screen_to_world(Vec2::ZERO, VIEWPORT)));
            prop_assert!(bounds.contains(camera.screen_to_world(VIEWPORT, VIEWPORT)));
        }
    }
}
