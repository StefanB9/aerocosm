//! The map camera: input handling around the pure [`controller`] maths.
//!
//! Two cameras share one view: the map camera draws the basemap (and its gizmo
//! lines), the overlay camera draws runways, airports and the UI on top. Bevy
//! draws 2D gizmos after all meshes of a camera, so splitting layers keeps
//! lines beneath airport symbols.

pub(crate) mod controller;

use bevy::{
    camera::{ClearColorConfig, visibility::RenderLayers},
    input::{
        mouse::{MouseScrollUnit, MouseWheel},
        touch::Touches,
    },
    picking::{hover::HoverMap, pointer::PointerId},
    prelude::*,
    ui::Node,
    window::PrimaryWindow,
};

use self::controller::{CameraLimits, CameraState};
use crate::{
    data::{MAP_LAYER, MapData, OVERLAY_LAYER},
    projection::MapProjection,
    theme,
};

/// Wheel lines to natural-log zoom: one notch zooms by ~1.3×.
const WHEEL_ZOOM_PER_LINE: f32 = 0.26;
/// Wheel pixels (touchpads) to natural-log zoom.
const WHEEL_ZOOM_PER_PIXEL: f32 = 0.003;
/// A press that moves less than this many pixels is a click, not a drag.
const CLICK_SLOP: f32 = 4.0;
/// Keyboard pan speed in pixels per second.
const KEY_PAN_SPEED: f32 = 600.0;

/// Germany's extent, for the initial view.
const HOME_SOUTH_WEST: (f64, f64) = (47.2, 5.8);
const HOME_NORTH_EAST: (f64, f64) = (55.1, 15.1);

/// Systems that update the camera and pointer state; others read them
/// afterwards.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CameraSet;

/// Marks both map cameras.
#[derive(Component)]
pub(crate) struct MapCamera;

/// Camera state, limits and the in-progress gesture.
#[derive(Resource)]
pub(crate) struct CameraRig {
    pub(crate) state: CameraState,
    pub(crate) limits: CameraLimits,
    home: Rect,
    initialised: bool,
    drag: Option<Drag>,
    pinch_distance: Option<f32>,
}

impl CameraRig {
    /// Whether the map is being dragged.
    pub(crate) const fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// A rig at `state`; the home view defaults to the whole of
    /// `limits.bounds`.
    pub(crate) fn new(state: CameraState, limits: CameraLimits) -> Self {
        Self {
            state,
            limits,
            home: limits.bounds,
            initialised: false,
            drag: None,
            pinch_distance: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    last: Vec2,
    travelled: f32,
    /// Smoothed pointer velocity in pixels per second, for inertia on release.
    velocity: Vec2,
}

/// Pointer facts other systems need this frame.
#[derive(Resource, Default, Debug)]
pub(crate) struct PointerState {
    /// Cursor position in window pixels, if over the window.
    pub(crate) cursor: Option<Vec2>,
    /// The primary button was released this frame without dragging.
    pub(crate) clicked: bool,
    /// The pointer is over a UI element, so the map ignores it.
    pub(crate) over_ui: bool,
    /// Logical window size.
    pub(crate) viewport: Vec2,
}

/// Spawns the cameras and drives them from mouse, touch and keyboard.
pub(crate) struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PointerState>()
            .add_systems(Startup, setup_cameras)
            .add_systems(
                Update,
                (track_pointer, handle_input, step_camera, apply_camera)
                    .chain()
                    .in_set(CameraSet),
            );
    }
}

fn setup_cameras(mut commands: Commands, projection: Res<MapProjection>, map: Res<MapData>) {
    let project = |(lat, lon): (f64, f64)| {
        aerocosm_core::LatLon::new(lat, lon).map(|p| projection.project(p))
    };
    let (Ok(south_west), Ok(north_east)) = (project(HOME_SOUTH_WEST), project(HOME_NORTH_EAST))
    else {
        return;
    };
    let home = Rect::from_corners(south_west, north_east);
    let bounds = map.basemap.bounds();
    let limits = CameraLimits {
        // The visible area stays on the basemap, so its edge is never seen.
        bounds: Rect::from_corners(
            projection.project(bounds.south_west()),
            projection.project(bounds.north_east()),
        ),
        min_scale: 0.4,
        max_scale: 2400.0,
    };
    commands.insert_resource(CameraRig {
        home,
        ..CameraRig::new(CameraState::new(home.center(), limits.max_scale), limits)
    });
    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(theme::SEA),
            ..default()
        },
        Msaa::Sample4,
        RenderLayers::layer(MAP_LAYER),
        MapCamera,
    ));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Msaa::Sample4,
        RenderLayers::layer(OVERLAY_LAYER),
        IsDefaultUiCamera,
        MapCamera,
    ));
}

fn track_pointer(
    window: Single<&Window, With<PrimaryWindow>>,
    hover: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut pointer: ResMut<PointerState>,
) {
    pointer.viewport = window.size();
    pointer.cursor = window.cursor_position();
    pointer.clicked = false;
    pointer.over_ui = hover
        .get(&PointerId::Mouse)
        .is_some_and(|hits| hits.keys().any(|entity| ui_nodes.contains(*entity)));
}

fn handle_input(
    mut rig: ResMut<CameraRig>,
    mut pointer: ResMut<PointerState>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    mut wheel: MessageReader<MouseWheel>,
    time: Res<Time>,
) {
    let viewport = pointer.viewport;
    if viewport.x <= 0.0 || viewport.y <= 0.0 {
        return;
    }
    let rig = &mut *rig;
    if !rig.initialised {
        rig.state = CameraState::fit(rig.home, viewport * 0.92, &rig.limits);
        rig.initialised = true;
    }
    let dt = time.delta_secs();

    // Wheel zoom towards the cursor.
    for event in wheel.read() {
        debug!(unit = ?event.unit, y = event.y, cursor = ?pointer.cursor, "wheel");
        if pointer.over_ui {
            continue;
        }
        let amount = match event.unit {
            MouseScrollUnit::Line => event.y * WHEEL_ZOOM_PER_LINE,
            MouseScrollUnit::Pixel => event.y * WHEEL_ZOOM_PER_PIXEL,
        };
        let anchor = pointer.cursor.unwrap_or(viewport / 2.0);
        rig.state.queue_zoom(-amount, anchor);
    }

    // Mouse drag with inertia; a press without movement is a click.
    if let Some(cursor) = pointer.cursor {
        if buttons.just_pressed(MouseButton::Left) && !pointer.over_ui {
            rig.drag = Some(Drag {
                last: cursor,
                travelled: 0.0,
                velocity: Vec2::ZERO,
            });
        }
        if let Some(drag) = rig.drag.as_mut() {
            let delta = cursor - drag.last;
            drag.last = cursor;
            drag.travelled += delta.length();
            if dt > 0.0 {
                drag.velocity = drag.velocity.lerp(delta / dt, 0.4);
            }
            if drag.travelled > CLICK_SLOP {
                rig.state.pan_by_pixels(delta, &rig.limits);
            }
        }
    }
    if buttons.just_released(MouseButton::Left)
        && let Some(drag) = rig.drag.take()
    {
        if drag.travelled > CLICK_SLOP {
            rig.state.release(drag.velocity);
        } else {
            pointer.clicked = true;
        }
    }

    // Touch: one finger pans, two fingers pinch.
    let mut fingers = touches.iter();
    match (fingers.next(), fingers.next()) {
        (Some(one), None) => {
            rig.state.pan_by_pixels(one.delta(), &rig.limits);
            rig.pinch_distance = None;
        }
        (Some(a), Some(b)) => {
            let distance = a.position().distance(b.position());
            let midpoint = (a.position() + b.position()) / 2.0;
            if let Some(previous) = rig.pinch_distance {
                rig.state
                    .pinch(previous, distance, midpoint, viewport, &rig.limits);
            }
            rig.pinch_distance = Some(distance);
        }
        _ => rig.pinch_distance = None,
    }

    // Keyboard: arrows/WASD pan, +/- zoom about the centre, Home resets.
    let mut pan = Vec2::ZERO;
    for (codes, direction) in [
        ([KeyCode::ArrowLeft, KeyCode::KeyA], Vec2::new(1.0, 0.0)),
        ([KeyCode::ArrowRight, KeyCode::KeyD], Vec2::new(-1.0, 0.0)),
        ([KeyCode::ArrowUp, KeyCode::KeyW], Vec2::new(0.0, 1.0)),
        ([KeyCode::ArrowDown, KeyCode::KeyS], Vec2::new(0.0, -1.0)),
    ] {
        if keys.any_pressed(codes) {
            pan += direction;
        }
    }
    if pan != Vec2::ZERO {
        rig.state
            .pan_by_pixels(pan.normalize() * KEY_PAN_SPEED * dt, &rig.limits);
    }
    if keys.any_just_pressed([KeyCode::Equal, KeyCode::NumpadAdd]) {
        rig.state.queue_zoom(-2.0_f32.ln(), viewport / 2.0);
    }
    if keys.any_just_pressed([KeyCode::Minus, KeyCode::NumpadSubtract]) {
        rig.state.queue_zoom(2.0_f32.ln(), viewport / 2.0);
    }
    if keys.just_pressed(KeyCode::Home) {
        rig.state = CameraState::fit(rig.home, viewport * 0.92, &rig.limits);
    }
}

fn step_camera(mut rig: ResMut<CameraRig>, pointer: Res<PointerState>, time: Res<Time>) {
    let rig = &mut *rig;
    rig.state
        .step(time.delta_secs(), pointer.viewport, &rig.limits);
}

fn apply_camera(
    rig: Res<CameraRig>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MapCamera>>,
) {
    for (mut transform, mut projection) in &mut cameras {
        transform.translation = rig.state.center().extend(transform.translation.z);
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = rig.state.scale();
        }
    }
}
