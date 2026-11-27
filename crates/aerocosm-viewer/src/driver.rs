//! Runs the simulation from the viewer's frame loop at the chosen speed.

use std::time::Duration;

use aerocosm_core::{Pacer, SimSpeed, SimTime, Simulation, TICK, WallClock, WallInstant};
use anyhow::{Context, Result};
use bevy::{
    ecs::{resource::Resource, schedule::SystemSet},
    platform::time::Instant,
    tasks::{AsyncComputeTaskPool, Task, TaskPool, block_on, poll_once},
};

/// At most this many ticks per frame; `Max` speed runs exactly this many…
const MAX_TICKS_PER_FRAME: u32 = 2000;
/// …or as many as fit in this much wall time, ms, leaving the rest of a
/// frame for drawing.
pub(crate) const FRAME_BUDGET_MS: u64 = 25;

/// Systems that advance the simulation; readers of its state run after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SimSet;

/// Systems that read the simulation: they run once it exists, after the
/// ground has loaded (plan M5 D132).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SimReady;

/// The simulation plus the pacer that decides how far to advance it each
/// frame. Ticks run in the background (plan M9 PR 5): while they do, the
/// simulation is away, and readers keep what they last showed.
#[derive(Resource)]
pub(crate) struct SimDriver {
    simulation: Option<Simulation>,
    pacer: Pacer,
    running: Option<Task<(Simulation, Result<Advance>)>>,
    /// Whether a frame has collected yet: until then, the systems that read
    /// the simulation once, as it appears, have not run, so it stays.
    read: bool,
}

/// A wall clock reading taken from Bevy's real-time clock for one frame.
/// What one frame's advance did, for the frame-time readout (plan M5 D145).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Advance {
    /// Ticks run.
    pub(crate) ticks: u32,
    /// Wall time they took.
    pub(crate) spent: Duration,
}

struct FrameClock(Duration);

impl WallClock for FrameClock {
    fn now(&self) -> WallInstant {
        WallInstant::from_elapsed(self.0)
    }
}

impl SimDriver {
    /// Drives `simulation` at the default speed.
    pub(crate) fn new(simulation: Simulation) -> Result<Self> {
        Ok(Self {
            simulation: Some(simulation),
            pacer: Pacer::new(TICK, MAX_TICKS_PER_FRAME)?,
            running: None,
            read: false,
        })
    }

    /// The simulation, for reading state; none while ticks run in the
    /// background.
    pub(crate) const fn simulation(&self) -> Option<&Simulation> {
        self.simulation.as_ref()
    }

    /// The current speed.
    pub(crate) const fn speed(&self) -> SimSpeed {
        self.pacer.speed()
    }

    /// How far real time has progressed into the next tick, `0..1` (D50).
    pub(crate) fn tick_fraction(&self) -> f64 {
        self.pacer.tick_fraction()
    }

    /// Changes the speed from the next frame on.
    pub(crate) const fn set_speed(&mut self, speed: SimSpeed) {
        self.pacer.set_speed(speed);
    }

    /// Runs `count` ticks regardless of speed.
    #[cfg(test)]
    pub(crate) fn run_ticks(&mut self, count: u32) -> Result<()> {
        self.simulation
            .as_mut()
            .context("the simulation is ticking")?
            .run_ticks(count)
    }

    /// Starts the ticks due by `elapsed`, the real time since the app
    /// started, in the background (plan M9 PR 5): the simulation goes with
    /// them until [`SimDriver::collect`] takes it back. Nothing starts while
    /// earlier ticks still run; the ticks owed meanwhile run next time.
    pub(crate) fn dispatch(&mut self, elapsed: Duration) {
        if self.running.is_some() || self.simulation.is_none() || !self.read {
            return;
        }
        let due = self.pacer.ticks_due(&FrameClock(elapsed));
        if due == 0 {
            return;
        }
        let Some(mut simulation) = self.simulation.take() else {
            return;
        };
        let pool = AsyncComputeTaskPool::get_or_init(TaskPool::default);
        self.running = Some(pool.spawn(async move {
            let start = Instant::now();
            let ran = run_due(&mut simulation, due, || start.elapsed());
            (simulation, ran)
        }));
    }

    /// The simulation back from the background with what its ticks did, once
    /// they have finished; `None` while they run or if none were started.
    pub(crate) fn collect(&mut self) -> Option<Result<Advance>> {
        self.read = true;
        let finished = block_on(poll_once(self.running.as_mut()?))?;
        self.running = None;
        let (simulation, ran) = finished;
        self.simulation = Some(simulation);
        Some(ran)
    }

    /// Runs the ticks due by `elapsed`, the real time since the app started,
    /// until they have taken [`FRAME_BUDGET_MS`] of wall time as `now`
    /// measures it; the rest are dropped. On a machine too slow for the
    /// chosen speed the simulation then runs slower, instead of each frame
    /// owing more ticks than the last (seen in the browser with the world).
    #[cfg(test)]
    pub(crate) fn advance(
        &mut self,
        elapsed: Duration,
        now: impl FnMut() -> Duration,
    ) -> Result<Advance> {
        let due = self.pacer.ticks_due(&FrameClock(elapsed));
        match self.simulation.as_mut() {
            Some(simulation) => run_due(simulation, due, now),
            None => Ok(Advance::default()),
        }
    }
}

/// Runs `due` ticks of `simulation` until they have taken
/// [`FRAME_BUDGET_MS`] of wall time as `now` measures it; the rest are
/// dropped.
fn run_due(
    simulation: &mut Simulation,
    due: u32,
    mut now: impl FnMut() -> Duration,
) -> Result<Advance> {
    let budget = Duration::from_millis(FRAME_BUDGET_MS);
    let start = now();
    let mut frame = Advance::default();
    for _ in 0..due {
        simulation
            .run_ticks(1)
            .with_context(|| format!("simulation stopped at {}", simulation.time()))?;
        frame.ticks += 1;
        frame.spent = now().saturating_sub(start);
        if frame.spent >= budget {
            break;
        }
    }
    Ok(frame)
}

/// A friendly clock such as `Thu 5 Nov 2026  06:07:08 UTC`.
pub(crate) fn format_clock(time: SimTime) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = time.as_millis().div_euclid(86_400_000);
    // 1970-01-01 was a Thursday.
    let weekday = WEEKDAYS[usize::try_from((days + 4).rem_euclid(7)).unwrap_or_default()];
    let civil = time.to_civil();
    let month = MONTHS[usize::from(civil.month().saturating_sub(1)).min(11)];
    format!(
        "{weekday} {} {month} {}  {:02}:{:02}:{:02} UTC",
        civil.day(),
        civil.year(),
        civil.hour(),
        civil.minute(),
        civil.second()
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aerocosm_core::{RngSeed, Scenario, SimConfig, SimSpeed, SimTime, Simulation};
    use anyhow::Result;

    use super::*;

    fn driver() -> Result<SimDriver> {
        let start: SimTime = "2026-11-05T06:00:00Z".parse()?;
        SimDriver::new(Simulation::new(
            SimConfig::new(start, RngSeed::new(1)),
            Scenario::empty(),
        )?)
    }

    /// A wall clock for measuring work that moves `step` on every reading.
    fn ticking(step: Duration) -> impl FnMut() -> Duration {
        let mut now = Duration::ZERO;
        move || {
            now += step;
            now
        }
    }

    #[test]
    fn test_driver_runs_at_default_speed() -> Result<()> {
        let mut driver = driver()?;
        assert_eq!(driver.speed(), SimSpeed::X60);
        driver.advance(Duration::ZERO, ticking(Duration::ZERO))?;
        driver.advance(Duration::from_secs(1), ticking(Duration::ZERO))?;
        assert_eq!(driver.simulation().context("simulation")?.ticks(), 60);
        Ok(())
    }

    #[test]
    fn test_driver_pause_and_speed_change() -> Result<()> {
        let mut driver = driver()?;
        driver.advance(Duration::ZERO, ticking(Duration::ZERO))?;
        driver.set_speed(SimSpeed::Paused);
        driver.advance(Duration::from_secs(5), ticking(Duration::ZERO))?;
        assert_eq!(driver.simulation().context("simulation")?.ticks(), 0);
        driver.set_speed(SimSpeed::X10);
        driver.advance(Duration::from_secs(6), ticking(Duration::ZERO))?;
        assert_eq!(driver.simulation().context("simulation")?.ticks(), 10);
        Ok(())
    }

    #[test]
    fn test_slow_ticks_stop_at_the_frame_budget() -> Result<()> {
        let mut driver = driver()?;
        driver.advance(Duration::ZERO, ticking(Duration::ZERO))?;
        // 60 ticks are due, but each takes 10 ms: the frame stops once the
        // budget is spent, and the rest are dropped, not owed.
        driver.advance(Duration::from_secs(1), ticking(Duration::from_millis(10)))?;
        let run = driver.simulation().context("simulation")?.ticks();
        assert!(run >= 1 && run * 10 <= 10 + FRAME_BUDGET_MS, "{run}");
        driver.advance(Duration::from_secs(2), ticking(Duration::ZERO))?;
        assert_eq!(driver.simulation().context("simulation")?.ticks(), run + 60);
        Ok(())
    }

    #[test]
    fn test_advance_reports_ticks_run_and_time_spent() -> Result<()> {
        let mut driver = driver()?;
        driver.advance(Duration::ZERO, ticking(Duration::ZERO))?;
        // Each reading of the frame clock is a millisecond later.
        let frame = driver.advance(Duration::from_secs(1), ticking(Duration::from_millis(1)))?;
        assert_eq!(frame.ticks, 25);
        assert_eq!(frame.spent, Duration::from_millis(25));
        driver.set_speed(SimSpeed::Paused);
        let idle = driver.advance(Duration::from_secs(2), ticking(Duration::from_millis(1)))?;
        assert_eq!((idle.ticks, idle.spent), (0, Duration::ZERO));
        Ok(())
    }

    /// Collects the background ticks, waiting for them.
    fn wait(driver: &mut SimDriver) -> Result<Advance> {
        let started = std::time::Instant::now();
        loop {
            if let Some(ran) = driver.collect() {
                return ran;
            }
            if started.elapsed() > Duration::from_secs(30) {
                anyhow::bail!("the background ticks did not finish");
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn test_dispatched_ticks_run_in_the_background() -> Result<()> {
        let mut driver = driver()?;
        driver.dispatch(Duration::ZERO);
        driver.dispatch(Duration::from_secs(1));
        assert!(
            driver.simulation().is_some(),
            "kept until a frame has read it"
        );
        let mut driver = self::driver()?;
        // A frame reads it; the pacer starts with nothing due.
        assert!(driver.collect().is_none());
        driver.dispatch(Duration::ZERO);
        assert!(driver.collect().is_none());
        driver.dispatch(Duration::from_secs(1));
        assert!(driver.simulation().is_none(), "away while it ticks");
        // A second dispatch while ticks run starts nothing.
        driver.dispatch(Duration::from_secs(2));
        let ran = wait(&mut driver)?;
        assert_eq!(ran.ticks, 60);
        let simulation = driver.simulation().context("back")?;
        assert_eq!(simulation.ticks(), 60);
        assert!(driver.collect().is_none(), "nothing more to collect");
        // The ticks owed meanwhile run with the next dispatch.
        driver.dispatch(Duration::from_secs(2));
        wait(&mut driver)?;
        assert_eq!(driver.simulation().context("back")?.ticks(), 120);
        Ok(())
    }

    #[test]
    fn test_paused_dispatch_keeps_the_simulation() -> Result<()> {
        let mut driver = driver()?;
        driver.set_speed(SimSpeed::Paused);
        driver.dispatch(Duration::ZERO);
        driver.dispatch(Duration::from_secs(5));
        assert!(driver.simulation().is_some());
        assert!(driver.collect().is_none());
        Ok(())
    }

    #[test]
    fn test_format_clock() -> Result<()> {
        let time: SimTime = "2026-11-05T06:07:08Z".parse()?;
        assert_eq!(format_clock(time), "Thu 5 Nov 2026  06:07:08 UTC");
        let sunday: SimTime = "1970-01-04T23:59:59Z".parse()?;
        assert_eq!(format_clock(sunday), "Sun 4 Jan 1970  23:59:59 UTC");
        Ok(())
    }
}
