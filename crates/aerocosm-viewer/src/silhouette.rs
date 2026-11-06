//! Aircraft silhouettes by size class (D33): outlines in units of fuselage
//! length, nose towards +y.

use aerocosm_core::{AircraftType, WakeCategory};
use bevy::math::Vec2;

/// The four silhouettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SizeClass {
    /// Turboprops and regional jets.
    Regional,
    /// Single-aisle jets.
    NarrowBody,
    /// Twin-aisle jets.
    WideBody,
    /// The A380 (wake category super).
    VeryLarge,
}

impl SizeClass {
    /// All classes, smallest first.
    pub(crate) const ALL: [Self; 4] = [
        Self::Regional,
        Self::NarrowBody,
        Self::WideBody,
        Self::VeryLarge,
    ];

    /// Position in [`Self::ALL`].
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// The silhouette an aircraft type is drawn with.
pub(crate) fn size_class(aircraft: &AircraftType) -> SizeClass {
    match aircraft.wake() {
        WakeCategory::Super => SizeClass::VeryLarge,
        WakeCategory::Heavy => SizeClass::WideBody,
        _ if !aircraft.is_freighter() && aircraft.seats() <= 100 => SizeClass::Regional,
        _ => SizeClass::NarrowBody,
    }
}

/// Right half of each outline from the nose to the tail: fuselage, wing,
/// tailplane. Spans follow the real length-to-span ratios.
const HALVES: [&[[f32; 2]]; 4] = [
    // ATR 72 / E190: straight wing.
    &[
        [0.0, 0.5],
        [0.035, 0.45],
        [0.05, 0.36],
        [0.05, 0.12],
        [0.5, 0.10],
        [0.5, 0.02],
        [0.05, 0.0],
        [0.045, -0.34],
        [0.16, -0.42],
        [0.16, -0.47],
        [0.025, -0.46],
        [0.0, -0.5],
    ],
    // A320: swept wing, span about the length.
    &[
        [0.0, 0.5],
        [0.04, 0.45],
        [0.055, 0.36],
        [0.055, 0.10],
        [0.475, -0.10],
        [0.475, -0.14],
        [0.055, -0.06],
        [0.05, -0.33],
        [0.17, -0.44],
        [0.17, -0.48],
        [0.03, -0.46],
        [0.0, -0.5],
    ],
    // A350: wider fuselage, more sweep.
    &[
        [0.0, 0.5],
        [0.05, 0.44],
        [0.07, 0.34],
        [0.07, 0.12],
        [0.485, -0.16],
        [0.485, -0.20],
        [0.07, -0.05],
        [0.06, -0.32],
        [0.19, -0.43],
        [0.19, -0.47],
        [0.035, -0.45],
        [0.0, -0.5],
    ],
    // A380: span exceeds the length.
    &[
        [0.0, 0.5],
        [0.06, 0.44],
        [0.085, 0.33],
        [0.085, 0.14],
        [0.55, -0.14],
        [0.55, -0.19],
        [0.085, -0.04],
        [0.075, -0.32],
        [0.21, -0.42],
        [0.21, -0.47],
        [0.04, -0.45],
        [0.0, -0.5],
    ],
];

/// The closed outline of `class`, counter-clockwise from the nose.
pub(crate) fn outline(class: SizeClass) -> Vec<Vec2> {
    let half = HALVES[class.index()];
    let mut points = Vec::with_capacity(half.len() * 2);
    points.extend(half.iter().map(|[x, y]| Vec2::new(*x, *y)));
    // The left side back from the tail; the nose and tail lie on the axis.
    points.extend(
        half[1..half.len() - 1]
            .iter()
            .rev()
            .map(|[x, y]| Vec2::new(-x, *y)),
    );
    points
}

/// Length on screen when zoomed out, in pixels.
pub(crate) const fn pixel_length(class: SizeClass) -> f32 {
    match class {
        SizeClass::Regional => 12.0,
        SizeClass::NarrowBody => 15.0,
        SizeClass::WideBody => 19.0,
        SizeClass::VeryLarge => 22.0,
    }
}

/// True fuselage length, used once the aircraft is larger to scale.
pub(crate) const fn real_length(class: SizeClass) -> f32 {
    match class {
        SizeClass::Regional => 30.0,
        SizeClass::NarrowBody => 38.0,
        SizeClass::WideBody => 65.0,
        SizeClass::VeryLarge => 73.0,
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::AircraftCatalog;
    use anyhow::{Context, Result};
    use bevy::math::Vec2;

    use super::*;

    fn catalog() -> Result<AircraftCatalog> {
        AircraftCatalog::from_json(include_str!("../../../assets/aircraft/types.json"))
    }

    fn class_of(catalog: &AircraftCatalog, code: &str) -> Result<SizeClass> {
        let id = catalog.by_code(code.parse()?).context("type")?;
        Ok(size_class(catalog.get(id).context("type")?))
    }

    #[test]
    fn test_size_class_by_type() -> Result<()> {
        let catalog = catalog()?;
        assert_eq!(class_of(&catalog, "AT76")?, SizeClass::Regional);
        assert_eq!(class_of(&catalog, "E190")?, SizeClass::Regional);
        assert_eq!(class_of(&catalog, "A320")?, SizeClass::NarrowBody);
        assert_eq!(class_of(&catalog, "B752")?, SizeClass::NarrowBody);
        assert_eq!(class_of(&catalog, "A359")?, SizeClass::WideBody);
        assert_eq!(class_of(&catalog, "B77L")?, SizeClass::WideBody);
        assert_eq!(class_of(&catalog, "A388")?, SizeClass::VeryLarge);
        Ok(())
    }

    #[test]
    fn test_outlines_are_symmetric_and_one_length_long() {
        for class in SizeClass::ALL {
            let outline = outline(class);
            let (low, high) = outline.iter().fold((f32::MAX, f32::MIN), |(low, high), p| {
                (low.min(p.y), high.max(p.y))
            });
            assert!((high - low - 1.0).abs() < 1e-6, "{class:?}");
            assert!((high - 0.5).abs() < 1e-6, "nose at +0.5");
            for point in &outline {
                let mirrored = Vec2::new(-point.x, point.y);
                assert!(
                    outline.iter().any(|p| p.distance(mirrored) < 1e-6),
                    "{class:?}"
                );
            }
        }
    }

    #[test]
    fn test_classes_grow_in_size() {
        for pair in SizeClass::ALL.windows(2) {
            assert!(pixel_length(pair[0]) < pixel_length(pair[1]));
            assert!(real_length(pair[0]) < real_length(pair[1]));
        }
    }

    #[test]
    fn test_silhouettes_tessellate() -> Result<()> {
        for class in SizeClass::ALL {
            let triangles = crate::tessellate::fill_outline(&outline(class))?;
            assert!(triangles.indices.len() >= 3 * 8, "{class:?}");
            assert_eq!(triangles.indices.len() % 3, 0);
        }
        Ok(())
    }
}
