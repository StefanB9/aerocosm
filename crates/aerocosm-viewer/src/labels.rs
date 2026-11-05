//! Greedy, collision-free label placement in screen space.

use bevy::math::{Rect, Vec2};

/// Minimum clearance between labels, in pixels.
const PADDING: f32 = 2.0;
/// How far a displaced centred label moves beyond its anchor, in pixels.
const CENTRED_NUDGE: f32 = 3.0;

/// A label that would like to be shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LabelCandidate {
    /// Screen position of the labelled feature.
    pub(crate) anchor: Vec2,
    /// Size of the rendered text.
    pub(crate) size: Vec2,
    /// Distance from the anchor for a label beside a symbol; `None` centres the
    /// label on the anchor (e.g. a city name).
    pub(crate) gap: Option<f32>,
}

impl LabelCandidate {
    /// Positions to try, in order of preference: beside a symbol right, left,
    /// above, below; a centred label on its anchor, then just above or below
    /// it.
    fn positions(&self) -> impl Iterator<Item = Rect> + '_ {
        let size = self.size;
        let (sides, gap): (&[Vec2], f32) = match self.gap {
            Some(gap) => (&[Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y], gap),
            None => (&[Vec2::ZERO, Vec2::Y, Vec2::NEG_Y], CENTRED_NUDGE),
        };
        sides.iter().map(move |side| {
            // Screen y grows downwards, so "above" is negative y.
            let centre = self.anchor
                + Vec2::new(
                    side.x * (gap + size.x / 2.0),
                    -side.y * (gap + size.y / 2.0),
                );
            Rect::from_center_size(centre, size)
        })
    }
}

/// Places candidates in order (most important first). Each label takes the
/// first of its positions that is fully on screen and clear of every label
/// placed before it, or is left out. Writes `(candidate index, screen rect)`
/// pairs into `placed`, reusing its allocation.
pub(crate) fn place_labels(
    candidates: &[LabelCandidate],
    viewport: Vec2,
    placed: &mut Vec<(usize, Rect)>,
) {
    placed.clear();
    let screen = Rect::from_corners(Vec2::ZERO, viewport);
    for (index, candidate) in candidates.iter().enumerate() {
        let free = candidate.positions().find(|rect| {
            screen.contains(rect.min)
                && screen.contains(rect.max)
                && placed
                    .iter()
                    .all(|(_, other)| other.intersect(rect.inflate(PADDING)).is_empty())
        });
        if let Some(rect) = free {
            placed.push((index, rect));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{Rect, Vec2};
    use proptest::prelude::*;

    use super::*;

    const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    fn candidate(x: f32, y: f32, width: f32) -> LabelCandidate {
        LabelCandidate {
            anchor: Vec2::new(x, y),
            size: Vec2::new(width, 14.0),
            gap: Some(6.0),
        }
    }

    fn centred(x: f32, y: f32, width: f32) -> LabelCandidate {
        LabelCandidate {
            gap: None,
            ..candidate(x, y, width)
        }
    }

    #[test]
    fn test_label_prefers_right_of_anchor() {
        let mut placed = Vec::new();
        place_labels(&[candidate(100.0, 100.0, 50.0)], VIEWPORT, &mut placed);
        assert_eq!(placed.len(), 1);
        let rect = placed[0].1;
        assert!(rect.min.x > 100.0 && (rect.center().y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn test_crowded_label_moves_to_another_side() {
        let mut placed = Vec::new();
        let candidates = [candidate(100.0, 100.0, 80.0), candidate(110.0, 102.0, 80.0)];
        place_labels(&candidates, VIEWPORT, &mut placed);
        assert_eq!(placed.len(), 2);
        assert!(placed[1].1.max.x < 110.0, "goes left: {:?}", placed[1].1);
    }

    #[test]
    fn test_earlier_candidates_win_when_no_side_fits() {
        let mut placed = Vec::new();
        let candidates = [
            centred(100.0, 100.0, 120.0),
            centred(105.0, 101.0, 120.0),
            centred(100.0, 300.0, 80.0),
        ];
        place_labels(&candidates, VIEWPORT, &mut placed);
        let indices: Vec<usize> = placed.iter().map(|(index, _)| *index).collect();
        assert_eq!(indices, [0, 2]);
    }

    #[test]
    fn test_centred_label_sits_on_anchor() {
        let mut placed = Vec::new();
        place_labels(&[centred(400.0, 300.0, 60.0)], VIEWPORT, &mut placed);
        assert_eq!(placed[0].1.center(), Vec2::new(400.0, 300.0));
    }

    #[test]
    fn test_crowded_centred_label_moves_above_its_anchor() {
        let mut placed = Vec::new();
        let candidates = [candidate(380.0, 308.0, 60.0), centred(400.0, 300.0, 60.0)];
        place_labels(&candidates, VIEWPORT, &mut placed);
        assert_eq!(placed.len(), 2);
        assert!(placed[1].1.max.y <= 300.0, "above: {:?}", placed[1].1);
    }

    #[test]
    fn test_label_at_screen_edge_flips_inside() {
        let mut placed = Vec::new();
        place_labels(&[candidate(790.0, 100.0, 80.0)], VIEWPORT, &mut placed);
        assert!(placed[0].1.max.x <= 790.0);
    }

    #[test]
    fn test_offscreen_labels_are_skipped() {
        let mut placed = Vec::new();
        place_labels(
            &[candidate(-50.0, 100.0, 20.0), centred(400.0, -40.0, 20.0)],
            VIEWPORT,
            &mut placed,
        );
        assert_eq!(placed.len(), 0);
    }

    #[test]
    fn test_output_is_cleared_between_frames() {
        let mut placed = vec![(7, Rect::new(0.0, 0.0, 1.0, 1.0))];
        place_labels(&[], VIEWPORT, &mut placed);
        assert_eq!(placed.len(), 0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_placed_labels_never_overlap_and_stay_on_screen(
            raw in prop::collection::vec((0.0_f32..800.0, 0.0_f32..600.0, 10.0_f32..120.0), 0..60),
        ) {
            let candidates: Vec<_> = raw.iter().map(|(x, y, w)| candidate(*x, *y, *w)).collect();
            let mut placed = Vec::new();
            place_labels(&candidates, VIEWPORT, &mut placed);
            let screen = Rect::from_corners(Vec2::ZERO, VIEWPORT);
            for (i, (_, a)) in placed.iter().enumerate() {
                prop_assert!(screen.contains(a.min) && screen.contains(a.max));
                for (_, b) in &placed[i + 1..] {
                    prop_assert!(a.intersect(*b).is_empty(), "{a:?} overlaps {b:?}");
                }
            }
        }

        #[test]
        fn test_first_visible_candidate_is_always_placed(x in 0.0_f32..600.0, y in 20.0_f32..580.0) {
            let mut placed = Vec::new();
            place_labels(&[candidate(x, y, 100.0), candidate(x, y, 100.0)], VIEWPORT, &mut placed);
            prop_assert!(!placed.is_empty());
            prop_assert_eq!(placed[0].0, 0);
            prop_assert!(placed[0].1.min.x > x, "first choice is right of the anchor");
        }
    }
}
