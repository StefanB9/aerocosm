//! The frame-time readout (plan M5 D145): F3 shows how long frames take,
//! how many ticks each runs and the simulation's share, so that the viewer
//! can be measured on real hardware. F4 turns vsync off and on (plan M9
//! D181), so frame times are not rounded to whole refresh intervals.

use std::{collections::VecDeque, time::Duration};

use bevy::{
    prelude::*,
    window::{PresentMode, PrimaryWindow},
};

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

    /// The readout's text, ending with whether frames wait for vsync.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn text(&self, vsync: bool) -> String {
        let vsync = if vsync { "vsync on" } else { "vsync off" };
        let count = self.frames.len();
        if count == 0 {
            return format!("frame – · {vsync}");
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
            "frame {:.1} ms avg, {:.1} worst · {:.1} ticks/frame · simulation {share:.0} % · \
             {vsync}",
            ms(total) / count as f64,
            ms(worst),
            ticks as f64 / count as f64,
        )
    }
}

/// Whether frames in `mode` wait for the display's refresh.
pub(crate) fn is_synced(mode: PresentMode) -> bool {
    matches!(
        mode,
        PresentMode::AutoVsync | PresentMode::Fifo | PresentMode::FifoRelaxed
    )
}

/// The mode F4 switches to from `mode`: vsync off if it was on, else on.
pub(crate) fn switched(mode: PresentMode) -> PresentMode {
    if is_synced(mode) {
        PresentMode::AutoNoVsync
    } else {
        PresentMode::AutoVsync
    }
}

/// Shows the readout on F3; switches vsync on F4.
pub(crate) struct ReadoutPlugin;

impl Plugin for ReadoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Advance>()
            .init_resource::<Recent>()
            .add_systems(Startup, spawn.after(crate::data::setup_fonts))
            .add_systems(Update, (toggle, switch_vsync, update).chain());
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

fn switch_vsync(
    keys: Res<ButtonInput<KeyCode>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    if keys.just_pressed(KeyCode::F4) {
        window.present_mode = switched(window.present_mode);
    }
}

fn update(
    time: Res<Time<Real>>,
    advance: Res<Advance>,
    mut recent: ResMut<Recent>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut readout: Single<(&mut Text, &Visibility), With<ReadoutText>>,
) {
    recent.0.push(time.delta(), advance.ticks, advance.spent);
    let (text, visibility) = &mut *readout;
    if **visibility != Visibility::Hidden {
        // Headless (benchmarks) there is no window; Bevy's default syncs.
        let synced = window.is_none_or(|window| is_synced(window.present_mode));
        let line = recent.0.text(synced);
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
        assert_eq!(times.text(true), "frame – · vsync on");
        times.push(ms(10), 1, ms(2));
        times.push(ms(30), 3, ms(10));
        assert_eq!(
            times.text(true),
            "frame 20.0 ms avg, 30.0 worst · 2.0 ticks/frame · simulation 30 % · vsync on"
        );
        assert_eq!(
            times.text(false),
            "frame 20.0 ms avg, 30.0 worst · 2.0 ticks/frame · simulation 30 % · vsync off"
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
            times.text(false),
            "frame 10.0 ms avg, 10.0 worst · 1.0 ticks/frame · simulation 50 % · vsync off"
        );
    }

    #[test]
    fn test_switched_turns_vsync_off_and_on() {
        for synced in [
            PresentMode::AutoVsync,
            PresentMode::Fifo,
            PresentMode::FifoRelaxed,
        ] {
            assert!(is_synced(synced));
            assert_eq!(switched(synced), PresentMode::AutoNoVsync);
        }
        for free in [
            PresentMode::AutoNoVsync,
            PresentMode::Immediate,
            PresentMode::Mailbox,
        ] {
            assert!(!is_synced(free));
            assert_eq!(switched(free), PresentMode::AutoVsync);
        }
    }
}
