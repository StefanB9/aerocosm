//! Static meshes and lines built in their own frame (plan M5 D130, D131):
//! vertices are offsets from an anchor, so they stay small in `f32`, and one
//! system places every anchor relative to the floating origin, at its copy
//! nearest the camera.

use bevy::{
    app::{App, Plugin, Update},
    ecs::{
        component::Component,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::{Query, Res},
    },
    math::Vec2,
    transform::components::Transform,
};

use crate::{
    camera::{CameraRig, CameraSet},
    projection::{MapProjection, WORLD_WIDTH_F32, wrap_near},
};

/// Where a mesh or line set built in `frame` is drawn: `copy` worlds east of
/// its copy nearest the camera.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub(crate) struct MapAnchor {
    frame: MapProjection,
    copy: i8,
}

impl MapAnchor {
    /// Drawn once, nearest the camera: for anything much smaller than the
    /// world.
    pub(crate) const fn new(frame: MapProjection) -> Self {
        Self { frame, copy: 0 }
    }

    /// Drawn three times, a world apart, so that a shape as wide as the
    /// world covers any view (the farthest zoom shows one world's width).
    pub(crate) const fn copies(frame: MapProjection) -> [Self; 3] {
        [
            Self { frame, copy: -1 },
            Self { frame, copy: 0 },
            Self { frame, copy: 1 },
        ]
    }

    /// The local position of the frame's origin with the camera at `centre`.
    pub(crate) fn place(&self, projection: &MapProjection, centre: Vec2) -> Vec2 {
        let world = WORLD_WIDTH_F32;
        wrap_near(projection.offset_to(&self.frame), centre)
            + Vec2::new(f32::from(self.copy) * world, 0.0)
    }
}

/// Systems that place anchored entities; readers of their transforms run
/// after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnchorSet;

/// Places anchored meshes and lines every frame.
pub(crate) struct AnchorPlugin;

impl Plugin for AnchorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, place_anchored.in_set(AnchorSet).after(CameraSet));
    }
}

fn place_anchored(
    projection: Res<MapProjection>,
    rig: Option<Res<CameraRig>>,
    mut anchored: Query<(&MapAnchor, &mut Transform)>,
) {
    let centre = rig.map_or(Vec2::ZERO, |rig| rig.state.center());
    for (anchor, mut transform) in &mut anchored {
        let at = anchor.place(&projection, centre);
        if transform.translation.truncate() != at {
            transform.translation = at.extend(transform.translation.z);
        }
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_anchors_are_placed_at_the_copy_nearest_the_camera() -> Result<()> {
        // Origin over Fiji, an anchor in Samoa: east, not a world away.
        let projection = MapProjection::new(LatLon::new(-17.8, 178.0)?);
        let samoa = MapAnchor::new(MapProjection::new(LatLon::new(-13.8, -172.0)?));
        let near = samoa.place(&projection, Vec2::ZERO);
        assert!(near.x > 0.0 && near.x < 1.2e6, "{near:?}");
        // With the camera half a world west, the western copy.
        let world = WORLD_WIDTH_F32;
        let far = samoa.place(&projection, Vec2::new(-0.6 * world, 0.0));
        assert!((far.x - (near.x - world)).abs() < 1.0, "{far:?}");
        Ok(())
    }

    #[test]
    fn test_three_copies_are_a_world_apart() -> Result<()> {
        let projection = MapProjection::new(LatLon::new(0.0, 0.0)?);
        let [west, middle, east] = MapAnchor::copies(projection);
        let [a, b, c] = [west, middle, east].map(|anchor| anchor.place(&projection, Vec2::ZERO));
        let world = WORLD_WIDTH_F32;
        assert!(b.length() < 1e-3);
        assert!((c.x - b.x - world).abs() < 1.0 && (b.x - a.x - world).abs() < 1.0);
        Ok(())
    }
}
