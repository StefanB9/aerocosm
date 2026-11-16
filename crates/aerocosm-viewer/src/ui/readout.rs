//! The frame-time readout (plan M5 D145): F3 shows how long frames take,
//! how many ticks each runs and the simulation's share, so that the viewer
//! can be measured on real hardware.

use std::{collections::VecDeque, time::Duration};

use bevy::prelude::*;

use super::{MapCover, text};
use crate::{data::Fonts, driver::Advance, theme};

/// Frames the readout averages over: about two seconds.
const WINDOW: usize = 120;

/// The last frames: their length, the ticks run and the time the simulation
/// took in them.
#[derive(Debug, Default)]
pub(crate) struct FrameTimes {
    frames: VecDeque<(Duration, u32, Duration)>,
}

impl FrameTimes {
    /// Adds a frame of length `frame` that ran `ticks` ticks taking `spent`.
    pub(crate) fn push(&mut self, frame: Duration, ticks: u32, spent: Duration) {
        if self.frames.len() == WINDOW {
            self.frames.pop_front();
        }
        self.frames.push_back((frame, ticks, spent));
    }

    /// The readout's text.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn text(&self) -> String {
        let count = self.frames.len();
        if count == 0 {
            return "frame –".to_owned();
        }
        let total: Duration = self.frames.iter().map(|(frame, _, _)| *frame).sum();
        let worst = self
            .frames
            .iter()
            .map(|(frame, _, _)| *frame)
            .max()
            .unwrap_or_default();
        let ticks: u64 = self
            .frames
            .iter()
            .map(|(_, ticks, _)| u64::from(*ticks))
            .sum();
        let spent: Duration = self.frames.iter().map(|(_, _, spent)| *spent).sum();
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        let share = if total.is_zero() {
            0.0
        } else {
            spent.as_secs_f64() / total.as_secs_f64() * 100.0
        };
        format!(
            "frame {:.1} ms avg, {:.1} worst · {:.1} ticks/frame · simulation {share:.0} %",
            ms(total) / count as f64,
            ms(worst),
            ticks as f64 / count as f64,
        )
    }
}

/// Shows the readout on F3.
pub(crate) struct ReadoutPlugin;

impl Plugin for ReadoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Advance>()
            .init_resource::<Recent>()
            .add_systems(Startup, spawn.after(crate::data::setup_fonts))
            .add_systems(Update, (toggle, update).chain());
    }
}

/// The frames seen so far.
#[derive(Resource, Default)]
struct Recent(FrameTimes);

#[derive(Component)]
struct ReadoutText;

fn spawn(mut commands: Commands, fonts: Res<Fonts>) {
    commands.spawn((
        MapCover,
        ReadoutText,
        Node {
            position_type: PositionType::Absolute,
            left: px(16),
            bottom: px(34),
            ..default()
        },
        Visibility::Hidden,
        text(&fonts, "", 12.0, theme::TEXT),
        Pickable::IGNORE,
    ));
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut readout: Single<&mut Visibility, With<ReadoutText>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        readout.toggle_visible_hidden();
    }
}

fn update(
    time: Res<Time<Real>>,
    advance: Res<Advance>,
    mut recent: ResMut<Recent>,
    mut readout: Single<(&mut Text, &Visibility), With<ReadoutText>>,
) {
    recent.0.push(time.delta(), advance.ticks, advance.spent);
    let (text, visibility) = &mut *readout;
    if **visibility != Visibility::Hidden {
        let line = recent.0.text();
        if text.0 != line {
            text.0 = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn test_readout_shows_average_worst_ticks_and_share() {
        let mut times = FrameTimes::default();
        assert_eq!(times.text(), "frame –");
        times.push(ms(10), 1, ms(2));
        times.push(ms(30), 3, ms(10));
        assert_eq!(
            times.text(),
            "frame 20.0 ms avg, 30.0 worst · 2.0 ticks/frame · simulation 30 %"
        );
    }

    #[test]
    fn test_readout_keeps_the_last_frames_only() {
        let mut times = FrameTimes::default();
        times.push(ms(500), 0, ms(0));
        for _ in 0..WINDOW {
            times.push(ms(10), 1, ms(5));
        }
        assert_eq!(
            times.text(),
            "frame 10.0 ms avg, 10.0 worst · 1.0 ticks/frame · simulation 50 %"
        );
    }
}
