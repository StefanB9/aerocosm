//! Pure logic of the aircraft layer: colours by phase (D49), level of
//! detail, interpolation between ticks (D50), sizes and trails (D51).

use std::collections::VecDeque;

use aerocosm_core::{SimTime, ops::Phase};
use bevy::{color::Color, math::Vec2};

/// Above this zoom (map units per pixel) aircraft on the ground are hidden.
const GROUND_SCALE: f32 = 150.0;
/// Above this zoom only the hovered and selected flight show trails (D51).
const TRAIL_SCALE: f32 = 1000.0;
/// Trails cover this many simulated seconds…
const TRAIL_SECONDS: f64 = 120.0;
/// …sampled this often.
const TRAIL_SAMPLE_SECONDS: f64 = 10.0;

/// Phases grouped by how they are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PhaseBand {
    /// Boarding, taxiing and on the runway.
    Ground,
    /// Climbing.
    Climb,
    /// Level at cruise.
    Cruise,
    /// Descending and on approach.
    Descent,
}

impl PhaseBand {
    /// All bands.
    pub(crate) const ALL: [Self; 4] = [Self::Ground, Self::Climb, Self::Cruise, Self::Descent];

    /// The band of a phase.
    pub(crate) const fn of(phase: Phase) -> Self {
        match phase {
            Phase::Climb => Self::Climb,
            Phase::Cruise => Self::Cruise,
            Phase::Holding | Phase::Descent | Phase::Approach => Self::Descent,
            Phase::Boarding
            | Phase::TaxiOut
            | Phase::Queue
            | Phase::TakeoffRoll
            | Phase::Landing
            | Phase::TaxiIn
            | Phase::Arrived => Self::Ground,
        }
    }

    /// Position in [`Self::ALL`].
    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The band's colour on the map (D49).
    pub(crate) const fn color(self) -> Color {
        match self {
            Self::Ground => Color::srgb(0.62, 0.66, 0.74),
            Self::Climb => Color::srgb(0.45, 0.85, 0.55),
            Self::Cruise => Color::srgb(0.55, 0.80, 1.00),
            Self::Descent => Color::srgb(0.98, 0.47, 0.56),
        }
    }
}

/// Below this relative luminance a brand colour is lightened for the dark map.
const MIN_LUMINANCE: f32 = 0.30;

/// Relative luminance of an sRGB colour (WCAG), `0..=1`.
fn luminance([r, g, b]: [u8; 3]) -> f32 {
    let linear = |channel: u8| {
        let c = f32::from(channel) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// An airline colour that reads on the dark theme: dark brand colours (such
/// as navy blues) are mixed towards white until bright enough, keeping their
/// hue.
pub(crate) fn legible(rgb: [u8; 3]) -> [u8; 3] {
    let mix = |rgb: [u8; 3], amount: f32| {
        rgb.map(|channel| {
            let c = f32::from(channel);
            // The mix stays within 0..=255.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mixed = (c + (255.0 - c) * amount).round() as u8;
            mixed
        })
    };
    (0..=20)
        .map(|step| mix(rgb, f32::from(u8::try_from(step).unwrap_or(20)) * 0.05))
        .find(|candidate| luminance(*candidate) >= MIN_LUMINANCE)
        .unwrap_or([255, 255, 255])
}

/// Whether a flight in `phase` is drawn at zoom `scale`.
pub(crate) fn flight_visible(phase: Phase, scale: f32) -> bool {
    PhaseBand::of(phase) != PhaseBand::Ground || scale < GROUND_SCALE
}

/// Whether a flight's trail is drawn at zoom `scale`.
pub(crate) fn trail_visible(scale: f32, highlighted: bool) -> bool {
    highlighted || scale < TRAIL_SCALE
}

/// Where an aircraft is drawn: map position and true course in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    pub(crate) position: Vec2,
    pub(crate) heading: f32,
}

/// The pose a fraction `t` of the way from `previous` to `current`, turning
/// the short way round.
pub(crate) fn blend(previous: Pose, current: Pose, t: f32) -> Pose {
    let turn = (current.heading - previous.heading + 540.0).rem_euclid(360.0) - 180.0;
    let heading = (previous.heading + turn * t).rem_euclid(360.0);
    Pose {
        position: previous.position.lerp(current.position, t),
        heading: if heading >= 360.0 { 0.0 } else { heading },
    }
}

/// Rotation about the screen normal for a true course: the silhouettes point
/// north (+y), and courses turn clockwise.
pub(crate) fn rotation(heading: f32) -> f32 {
    -heading.to_radians()
}

/// Drawn length in map units: `pixels` on screen, but never smaller than the
/// true `meters` once zoomed in.
pub(crate) fn display_length(pixels: f32, meters: f32, scale: f32, units_per_meter: f32) -> f32 {
    (pixels * scale).max(meters * units_per_meter)
}

/// The recent track of one aircraft.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Trail {
    samples: VecDeque<(SimTime, Vec2)>,
    newest: Option<SimTime>,
}

impl Trail {
    /// Records `position` at `time` if a sample is due, forgetting old ones.
    pub(crate) fn push(&mut self, time: SimTime, position: Vec2) {
        let due = self.samples.back().is_none_or(|(last, _)| {
            time.duration_since(*last).as_secs_f64() >= TRAIL_SAMPLE_SECONDS
        });
        if due {
            self.samples.push_back((time, position));
        }
        while self
            .samples
            .front()
            .is_some_and(|(first, _)| time.duration_since(*first).as_secs_f64() > TRAIL_SECONDS)
        {
            self.samples.pop_front();
        }
        self.newest = Some(time);
    }

    /// Samples oldest first with their age as a fraction of the trail length.
    pub(crate) fn points(&self) -> impl Iterator<Item = (Vec2, f32)> + '_ {
        let newest = self.newest;
        self.samples.iter().map(move |(time, position)| {
            let age = newest.map_or(0.0, |newest| newest.duration_since(*time).as_secs_f64());
            // Ages are at most two minutes.
            #[allow(clippy::cast_possible_truncation)]
            let fraction = (age / TRAIL_SECONDS) as f32;
            (*position, fraction)
        })
    }

    /// Forgets the track, e.g. when a new leg starts.
    pub(crate) fn clear(&mut self) {
        self.samples.clear();
        self.newest = None;
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{SimDuration, SimTime};
    use anyhow::Result;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_phase_bands() {
        assert_eq!(PhaseBand::of(Phase::Boarding), PhaseBand::Ground);
        assert_eq!(PhaseBand::of(Phase::TakeoffRoll), PhaseBand::Ground);
        assert_eq!(PhaseBand::of(Phase::Climb), PhaseBand::Climb);
        assert_eq!(PhaseBand::of(Phase::Cruise), PhaseBand::Cruise);
        assert_eq!(PhaseBand::of(Phase::Approach), PhaseBand::Descent);
        assert_eq!(PhaseBand::of(Phase::TaxiIn), PhaseBand::Ground);
        assert!(
            PhaseBand::ALL
                .iter()
                .enumerate()
                .all(|(i, band)| band.index() == i)
        );
        let colours: Vec<_> = PhaseBand::ALL.iter().map(|band| band.color()).collect();
        for (i, a) in colours.iter().enumerate() {
            assert!(colours[i + 1..].iter().all(|b| b != a), "distinct colours");
        }
    }

    #[test]
    fn test_legible_lightens_only_dark_colours() {
        let navy = [5, 22, 77];
        let lightened = legible(navy);
        assert!(luminance(lightened) >= MIN_LUMINANCE);
        assert!(lightened[2] > lightened[0], "keeps the blue hue");
        let yellow = [255, 204, 0];
        assert_eq!(legible(yellow), yellow);
    }

    #[test]
    fn test_ground_traffic_only_when_zoomed_in() {
        assert!(flight_visible(Phase::Cruise, 3000.0));
        assert!(!flight_visible(Phase::TaxiOut, 1600.0));
        assert!(flight_visible(Phase::TaxiOut, 50.0));
    }

    #[test]
    fn test_trails_follow_zoom_and_selection() {
        assert!(!trail_visible(1600.0, false));
        assert!(trail_visible(1600.0, true));
        assert!(trail_visible(400.0, false));
    }

    #[test]
    fn test_blend_moves_between_poses() {
        let previous = Pose {
            position: Vec2::ZERO,
            heading: 350.0,
        };
        let current = Pose {
            position: Vec2::new(10.0, 0.0),
            heading: 10.0,
        };
        let half = blend(previous, current, 0.5);
        assert!(half.position.distance(Vec2::new(5.0, 0.0)) < 1e-5);
        // The short way round through north.
        assert!(half.heading.abs() < 1e-4 || (half.heading - 360.0).abs() < 1e-4);
        assert_eq!(blend(previous, current, 1.0).position, current.position);
    }

    #[test]
    fn test_rotation_turns_clockwise_from_north() {
        assert!(rotation(0.0).abs() < 1e-6);
        assert!((rotation(90.0) + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn test_display_length_grows_to_scale_when_zoomed_in() {
        // Zoomed out: constant pixels.
        assert!((display_length(15.0, 38.0, 1000.0, 1.6) - 15_000.0).abs() < 1e-3);
        // Zoomed in to 0.5 units per pixel: the true 38 m length wins.
        assert!((display_length(15.0, 38.0, 0.5, 1.6) - 60.8).abs() < 1e-3);
    }

    #[test]
    fn test_trail_samples_and_forgets() -> Result<()> {
        let start: SimTime = "2026-11-05T07:00:00Z".parse()?;
        let mut trail = Trail::default();
        for second in 0..300 {
            let time = start.checked_add(SimDuration::from_secs(second))?;
            trail.push(time, Vec2::new(f32::from(u16::try_from(second)?), 0.0));
        }
        let points: Vec<_> = trail.points().collect();
        // One sample every 10 s for the last 2 minutes.
        assert!((12..=13).contains(&points.len()), "{}", points.len());
        assert!(points.first().is_some_and(|(_, age)| *age > 0.9));
        assert!(
            points.windows(2).all(|pair| pair[0].1 > pair[1].1),
            "oldest first"
        );
        trail.clear();
        assert_eq!(trail.points().count(), 0);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_blend_heading_stays_between_on_the_short_arc(
            a in 0.0_f32..360.0, b in 0.0_f32..360.0, t in 0.0_f32..=1.0,
        ) {
            let previous = Pose { position: Vec2::ZERO, heading: a };
            let current = Pose { position: Vec2::ZERO, heading: b };
            let blended = blend(previous, current, t).heading;
            prop_assert!((0.0..360.0).contains(&blended));
            let arc = |x: f32, y: f32| { let d = (x - y).rem_euclid(360.0); d.min(360.0 - d) };
            prop_assert!(arc(a, blended) + arc(blended, b) <= arc(a, b) + 1e-2);
        }
    }
}
