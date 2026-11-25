//! Frame times in the real window (plan M9 D182): `--measure` plays a script
//! of scenes, each at a place and a simulation speed with vsync off; it
//! records every frame's length, ticks and simulation time, prints the
//! results as benchmark lines and closes the viewer.

use std::{io::Write as _, time::Duration};

use aerocosm_core::{LatLon, SimSpeed};
use bevy::{
    app::{App, AppExit, Plugin, Update},
    ecs::{
        message::MessageWriter,
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Res, ResMut, Single},
    },
    log::error,
    math::Vec2,
    time::{Real, Time},
    window::{PresentMode, PrimaryWindow, Window},
};

use crate::{
    camera::{CameraRig, PointerState, controller::CameraState},
    driver::{Advance, SimDriver, SimReady, SimSet},
    projection::MapProjection,
    start::Measure,
};

/// Where a scene looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Place {
    /// The whole world, as the viewer opens.
    World,
    /// A point at a scale in metres per pixel.
    At { lat: f64, lon: f64, scale: f32 },
}

/// One measured scene: a place and a simulation speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scene {
    pub(crate) name: &'static str,
    pub(crate) place: Place,
    pub(crate) speed: SimSpeed,
}

/// Frankfurt's stands, as in the F3 readings of plan M9.
const FRANKFURT: Place = Place::At {
    lat: 50.033,
    lon: 8.570,
    scale: 0.8,
};

/// The scenes, in the order they are played.
pub(crate) const SCENES: [Scene; 5] = [
    Scene {
        name: "world-60x",
        place: Place::World,
        speed: SimSpeed::X60,
    },
    Scene {
        name: "world-paused",
        place: Place::World,
        speed: SimSpeed::Paused,
    },
    Scene {
        name: "world-max",
        place: Place::World,
        speed: SimSpeed::Max,
    },
    Scene {
        name: "frankfurt-60x",
        place: FRANKFURT,
        speed: SimSpeed::X60,
    },
    Scene {
        name: "frankfurt-paused",
        place: FRANKFURT,
        speed: SimSpeed::Paused,
    },
];

/// What the viewer does after a frame of the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Go to scene `n`: its place and speed.
    Enter(usize),
    /// Nothing changes.
    Continue,
    /// Every scene is measured.
    Done,
}

/// Where the script stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Not started.
    Waiting,
    /// In scene `scene`, entered at `entered`.
    Running { scene: usize, entered: Duration },
    /// Finished.
    Done,
}

/// The measurement script: scenes played in turn, each settled and then
/// measured.
#[derive(Debug, Resource)]
pub(crate) struct Script {
    timing: Measure,
    stage: Stage,
    frames: Vec<f64>,
    ticks: u64,
    spent: Duration,
    lines: Vec<String>,
}

/// Milliseconds, exact for whole ones.
#[allow(clippy::cast_precision_loss)]
fn millis(duration: Duration) -> f64 {
    duration.as_nanos() as f64 / 1e6
}

/// `value` rounded to `places` decimals.
fn rounded(value: f64, places: i32) -> f64 {
    let factor = 10_f64.powi(places);
    (value * factor).round() / factor
}

impl Script {
    /// A script not yet started.
    pub(crate) const fn new(timing: Measure) -> Self {
        Self {
            timing,
            stage: Stage::Waiting,
            frames: Vec::new(),
            ticks: 0,
            spent: Duration::ZERO,
            lines: Vec::new(),
        }
    }

    /// Takes a frame of length `frame` ending at `now` (real time), which ran
    /// `advance`'s ticks; says what to do next.
    pub(crate) fn step(&mut self, now: Duration, frame: Duration, advance: Advance) -> Step {
        match self.stage {
            Stage::Waiting => self.enter(0, now),
            Stage::Done => Step::Done,
            Stage::Running { scene, entered } => {
                let settled = entered + self.timing.settle;
                if now <= settled {
                    return Step::Continue;
                }
                self.frames.push(millis(frame));
                self.ticks += u64::from(advance.ticks);
                self.spent += advance.spent;
                if now < settled + self.timing.span {
                    return Step::Continue;
                }
                self.report(scene);
                self.enter(scene + 1, now)
            }
        }
    }

    fn enter(&mut self, scene: usize, now: Duration) -> Step {
        self.frames.clear();
        self.ticks = 0;
        self.spent = Duration::ZERO;
        if scene < SCENES.len() {
            self.stage = Stage::Running {
                scene,
                entered: now,
            };
            Step::Enter(scene)
        } else {
            self.stage = Stage::Done;
            Step::Done
        }
    }

    /// Adds scene `scene`'s results to the lines.
    #[allow(clippy::cast_precision_loss)]
    fn report(&mut self, scene: usize) {
        let name = SCENES[scene].name;
        let count = self.frames.len();
        let total: f64 = self.frames.iter().sum();
        let Some([median, p95, max, mean]) = crate::bench::summary(&mut self.frames) else {
            return;
        };
        for (measure, value) in [
            ("frame_median", median),
            ("frame_p95", p95),
            ("frame_max", max),
            ("frame_mean", mean),
        ] {
            let value = rounded(value, 3);
            self.lines
                .push(format!("bench window/{name}/{measure} {value} ms"));
        }
        let ticks = rounded(self.ticks as f64 / count as f64, 2);
        self.lines
            .push(format!("bench window/{name}/ticks_per_frame {ticks} count"));
        let share = if total > 0.0 {
            rounded(millis(self.spent) / total * 100.0, 0)
        } else {
            0.0
        };
        self.lines.push(format!(
            "bench window/{name}/simulation_share {share} percent"
        ));
    }

    /// The results so far, one benchmark line each.
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }
}

/// Plays the script when the viewer was started with `--measure`.
pub(crate) struct MeasurePlugin(pub(crate) Measure);

impl Plugin for MeasurePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Script::new(self.0))
            .add_systems(Update, play.after(SimSet).in_set(SimReady));
    }
}

#[allow(clippy::too_many_arguments)]
fn play(
    time: Res<Time<Real>>,
    advance: Res<Advance>,
    projection: Res<MapProjection>,
    pointer: Res<PointerState>,
    mut script: ResMut<Script>,
    mut driver: ResMut<SimDriver>,
    rig: Option<ResMut<CameraRig>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    match script.step(time.elapsed(), time.delta(), *advance) {
        Step::Continue => {}
        Step::Enter(scene) => {
            let Some(scene) = SCENES.get(scene) else {
                return;
            };
            window.present_mode = PresentMode::AutoNoVsync;
            driver.set_speed(scene.speed);
            if let Some(mut rig) = rig {
                rig.state = match scene.place {
                    Place::World => rig.home_view(pointer.viewport),
                    Place::At { lat, lon, scale } => match LatLon::new(lat, lon) {
                        Ok(point) => CameraState::new(projection.project(point), scale),
                        Err(error) => {
                            error!("measured place: {error:#}");
                            CameraState::new(Vec2::ZERO, scale)
                        }
                    },
                };
            }
        }
        Step::Done => {
            let mut out = std::io::stdout().lock();
            for line in script.lines() {
                if let Err(error) = writeln!(out, "{line}") {
                    error!("cannot print the measurement: {error}");
                }
            }
            exit.write(AppExit::Success);
        }
    }
}
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn advance(ticks: u32, spent: u64) -> Advance {
        Advance {
            ticks,
            spent: ms(spent),
        }
    }

    #[test]
    fn test_script_settles_measures_and_reports_each_scene() {
        let timing = Measure {
            settle: ms(100),
            span: ms(200),
        };
        let mut script = Script::new(timing);
        assert_eq!(script.step(ms(0), ms(10), advance(0, 0)), Step::Enter(0));
        // Settling: nothing recorded.
        assert_eq!(script.step(ms(50), ms(50), advance(9, 9)), Step::Continue);
        // Measured from 100 ms until the first frame at or past 300 ms.
        for now in [150, 200, 250] {
            assert_eq!(script.step(ms(now), ms(10), advance(2, 4)), Step::Continue);
        }
        assert_eq!(script.step(ms(320), ms(30), advance(2, 4)), Step::Enter(1));
        let mut step = Step::Continue;
        let mut now = 320;
        while step != Step::Done {
            now += 20;
            step = script.step(ms(now), ms(20), advance(0, 0));
            assert!(now < 10_000, "the script ends");
        }
        let lines = script.lines();
        let first = SCENES[0].name;
        assert!(lines.contains(&format!("bench window/{first}/frame_median 10 ms")));
        assert!(lines.contains(&format!("bench window/{first}/frame_max 30 ms")));
        assert!(lines.contains(&format!("bench window/{first}/frame_p95 30 ms")));
        assert!(lines.contains(&format!("bench window/{first}/ticks_per_frame 2 count")));
        assert!(lines.contains(&format!("bench window/{first}/simulation_share 27 percent")));
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.ends_with("frame_p95 20 ms"))
                .count(),
            SCENES.len() - 1,
            "every other scene ran at 20 ms"
        );
    }

    #[test]
    fn test_scenes_cover_the_world_and_frankfurt() {
        let names: Vec<_> = SCENES.iter().map(|scene| scene.name).collect();
        assert_eq!(
            names,
            [
                "world-60x",
                "world-paused",
                "world-max",
                "frankfurt-60x",
                "frankfurt-paused"
            ]
        );
    }
}
