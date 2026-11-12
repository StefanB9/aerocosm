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

/// Width of the dark halo round a silhouette, as a fraction of its length:
/// it sets the aircraft off whatever it is over (runway, stands, taxiways).
pub(crate) const HALO_WIDTH: f32 = 0.035;
/// The halo while an aircraft is small on screen, as a fraction of its
/// length: wide enough to show at all.
pub(crate) const WIDE_HALO_WIDTH: f32 = 0.125;
/// At least this many pixels of halo.
const MIN_HALO_PIXELS: f32 = 1.5;

/// The halo for an aircraft `pixels` long on screen: the thin one, or the
/// wide one while the thin one would be under `MIN_HALO_PIXELS`.
pub(crate) fn halo_width(pixels: f32) -> f32 {
    if pixels * HALO_WIDTH < MIN_HALO_PIXELS {
        WIDE_HALO_WIDTH
    } else {
        HALO_WIDTH
    }
}

/// A halo corner moves out at most this many widths, so that sharp corners
/// (nose, wing tips) do not spike.
const MITER_LIMIT: f32 = 2.0;

/// `outline` grown outwards by `width` all round (in either winding), for a
/// halo behind the silhouette.
pub(crate) fn halo(outline: &[Vec2], width: f32) -> Vec<Vec2> {
    let n = outline.len();
    // Twice the signed area: positive counter-clockwise.
    let area: f32 = (0..n)
        .map(|k| outline[k].perp_dot(outline[(k + 1) % n]))
        .sum();
    let outward = |a: Vec2, b: Vec2| {
        let edge = (b - a).normalize_or_zero();
        let normal = Vec2::new(edge.y, -edge.x);
        if area >= 0.0 { normal } else { -normal }
    };
    (0..n)
        .map(|k| {
            let (prev, here, next) = (outline[(k + n - 1) % n], outline[k], outline[(k + 1) % n]);
            let (a, b) = (outward(prev, here), outward(here, next));
            let mid = (a + b).normalize_or_zero();
            let cos = mid.dot(a).max(1.0 / MITER_LIMIT);
            here + mid * (width / cos)
        })
        .collect()
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

    /// Whether `p` lies inside the closed `outline` (ray casting).
    fn inside(outline: &[Vec2], p: Vec2) -> bool {
        let mut inside = false;
        for (k, a) in outline.iter().enumerate() {
            let b = outline[(k + 1) % outline.len()];
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }

    #[test]
    fn test_a_halo_grows_an_outline_evenly_either_way_round() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        for outline in [square.to_vec(), square.iter().rev().copied().collect()] {
            let grown = halo(&outline, 0.1);
            assert_eq!(grown.len(), 4);
            for corner in &grown {
                // Corners move out diagonally by the width on both axes.
                assert!((corner.x.abs().min((corner.x - 1.0).abs()) - 0.1).abs() < 1e-5);
                assert!((corner.y.abs().min((corner.y - 1.0).abs()) - 0.1).abs() < 1e-5);
                assert!(!inside(&outline, *corner));
            }
        }
    }

    #[test]
    fn test_every_silhouette_has_a_halo_round_it() -> Result<()> {
        for class in SizeClass::ALL {
            let shape = outline(class);
            let grown = halo(&shape, HALO_WIDTH);
            assert_eq!(grown.len(), shape.len());
            // The halo encloses the silhouette and fills as one shape.
            for point in &shape {
                assert!(inside(&grown, *point * 0.999), "{class:?} {point}");
            }
            let triangles = crate::tessellate::fill_outline(&grown)?;
            assert_ne!(triangles.indices.len(), 0);
        }
        Ok(())
    }

    #[test]
    fn test_small_aircraft_get_a_wider_halo() {
        // A narrow-body zoomed out (15 px): about 2 px of halo.
        assert!((1.5..=2.0).contains(&(halo_width(15.0) * 15.0)));
        // Large on screen: the thin halo.
        assert!((halo_width(200.0) - HALO_WIDTH).abs() < 1e-6);
        // At least 1.5 px at any size.
        for pixels in [12.0_f32, 15.0, 30.0, 42.0, 43.0, 60.0, 120.0, 400.0] {
            assert!(halo_width(pixels) * pixels >= 1.5 - 1e-3, "{pixels}");
        }
        // The wide halo fills as one shape round every silhouette too.
        for class in SizeClass::ALL {
            let shape = outline(class);
            let grown = halo(&shape, WIDE_HALO_WIDTH);
            for point in &shape {
                assert!(inside(&grown, *point * 0.999), "{class:?} {point}");
            }
        }
    }
}
