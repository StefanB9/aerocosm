//! Converting elapsed wall-clock time into simulation ticks.
//!
//! Core never reads a real clock: `std::time::Instant` is unavailable on wasm,
//! and tests must be deterministic. Front ends implement [`WallClock`] over
//! their own time source.

use std::{fmt, time::Duration};

use anyhow::Result;

use crate::{CoreError, SimDuration};

/// A monotonic wall-clock reading: time elapsed since an arbitrary, fixed
/// origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallInstant(Duration);

impl WallInstant {
    /// The instant `elapsed` after the clock's origin.
    pub const fn from_elapsed(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Time from `earlier` to `self`, or zero if `earlier` is later.
    pub const fn saturating_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// A monotonic source of wall-clock time, implemented by each front end.
pub trait WallClock {
    /// The current reading.
    fn now(&self) -> WallInstant;
}

/// How fast simulated time runs relative to wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SimSpeed {
    /// Simulated time stands still.
    Paused,
    /// Real time.
    X1,
    /// Ten times real time.
    X10,
    /// One simulated hour per real minute; a day passes in 24 minutes.
    #[default]
    X60,
    /// Five simulated minutes per real second.
    X300,
    /// About a quarter of an hour per real second.
    X1000,
    /// As many ticks as the frame budget allows.
    Max,
}

impl SimSpeed {
    const LADDER: [Self; 7] = [
        Self::Paused,
        Self::X1,
        Self::X10,
        Self::X60,
        Self::X300,
        Self::X1000,
        Self::Max,
    ];

    /// Simulated seconds per wall-clock second; `None` for [`SimSpeed::Max`].
    pub const fn multiplier(self) -> Option<u32> {
        match self {
            Self::Paused => Some(0),
            Self::X1 => Some(1),
            Self::X10 => Some(10),
            Self::X60 => Some(60),
            Self::X300 => Some(300),
            Self::X1000 => Some(1000),
            Self::Max => None,
        }
    }

    fn position(self) -> usize {
        Self::LADDER
            .iter()
            .position(|speed| *speed == self)
            .unwrap_or_default()
    }

    /// The next faster speed, saturating at [`SimSpeed::Max`].
    #[must_use]
    pub fn faster(self) -> Self {
        Self::LADDER[(self.position() + 1).min(Self::LADDER.len() - 1)]
    }

    /// The next slower speed, saturating at [`SimSpeed::Paused`].
    #[must_use]
    pub fn slower(self) -> Self {
        Self::LADDER[self.position().saturating_sub(1)]
    }
}

impl fmt::Display for SimSpeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.multiplier() {
            Some(0) => f.write_str("paused"),
            Some(multiplier) => write!(f, "{multiplier}×"),
            None => f.write_str("max"),
        }
    }
}

/// Decides how many fixed ticks to run each frame.
///
/// Fractional ticks carry over between frames, so over time the simulation
/// advances exactly `elapsed × speed`. When a frame would exceed
/// `max_ticks_per_frame` the excess is dropped instead of queued: a slow
/// machine runs the simulation slower rather than falling ever further behind
/// (the "spiral of death").
#[derive(Debug, Clone)]
pub struct Pacer {
    tick_nanos: u128,
    max_ticks_per_frame: u32,
    speed: SimSpeed,
    last: Option<WallInstant>,
    carry_nanos: u128,
}

impl Pacer {
    /// A pacer for ticks of length `tick`, running at the default speed.
    pub fn new(tick: SimDuration, max_ticks_per_frame: u32) -> Result<Self> {
        let invalid = |reason| CoreError::InvalidPacerConfig { reason };
        let tick_millis =
            u128::try_from(tick.as_millis()).map_err(|_| invalid("tick must be positive"))?;
        if tick_millis == 0 {
            return Err(invalid("tick must be positive").into());
        }
        if max_ticks_per_frame == 0 {
            return Err(invalid("max ticks per frame must be positive").into());
        }
        Ok(Self {
            tick_nanos: tick_millis * 1_000_000,
            max_ticks_per_frame,
            speed: SimSpeed::default(),
            last: None,
            carry_nanos: 0,
        })
    }

    /// The current speed.
    pub const fn speed(&self) -> SimSpeed {
        self.speed
    }

    /// Changes the speed from the next frame on.
    pub const fn set_speed(&mut self, speed: SimSpeed) {
        self.speed = speed;
    }

    /// Number of ticks to run this frame. Call once per frame.
    pub fn ticks_due(&mut self, clock: &impl WallClock) -> u32 {
        let now = clock.now();
        let elapsed = self
            .last
            .map_or(Duration::ZERO, |last| now.saturating_since(last));
        self.last = Some(now);
        let Some(multiplier) = self.speed.multiplier() else {
            self.carry_nanos = 0;
            return self.max_ticks_per_frame;
        };
        if multiplier == 0 {
            self.carry_nanos = 0;
            return 0;
        }
        let owed = self.carry_nanos + elapsed.as_nanos() * u128::from(multiplier);
        let ticks = owed / self.tick_nanos;
        match u32::try_from(ticks) {
            Ok(ticks) if ticks <= self.max_ticks_per_frame => {
                self.carry_nanos = owed % self.tick_nanos;
                ticks
            }
            _ => {
                self.carry_nanos = 0;
                self.max_ticks_per_frame
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, time::Duration};

    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    /// A clock the test advances by hand.
    struct ManualClock(Cell<Duration>);

    impl ManualClock {
        fn new() -> Self {
            Self(Cell::new(Duration::ZERO))
        }

        fn advance(&self, by: Duration) {
            self.0.set(self.0.get() + by);
        }
    }

    impl WallClock for ManualClock {
        fn now(&self) -> WallInstant {
            WallInstant::from_elapsed(self.0.get())
        }
    }

    fn pacer(max_ticks: u32) -> Result<Pacer> {
        Pacer::new(SimDuration::from_secs(1), max_ticks)
    }

    #[test]
    fn test_pacer_new_rejects_invalid_config() -> Result<()> {
        for (tick, max) in [
            (SimDuration::ZERO, 10),
            (SimDuration::from_millis(-1), 10),
            (SimDuration::from_secs(1), 0),
        ] {
            let error = Pacer::new(tick, max).err().context("must reject")?;
            assert!(matches!(
                error.downcast_ref::<CoreError>(),
                Some(CoreError::InvalidPacerConfig { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn test_pacer_default_speed_is_60x() -> Result<()> {
        assert_eq!(pacer(100)?.speed(), SimSpeed::X60);
        assert_eq!(SimSpeed::default(), SimSpeed::X60);
        Ok(())
    }

    #[test]
    fn test_pacer_first_frame_yields_no_ticks() -> Result<()> {
        let clock = ManualClock::new();
        clock.advance(Duration::from_secs(100));
        assert_eq!(pacer(1000)?.ticks_due(&clock), 0);
        Ok(())
    }

    #[test]
    fn test_pacer_ticks_scale_with_speed() -> Result<()> {
        let clock = ManualClock::new();
        for (speed, expected) in [
            (SimSpeed::X1, 1),
            (SimSpeed::X10, 10),
            (SimSpeed::X60, 60),
            (SimSpeed::X300, 300),
            (SimSpeed::X1000, 1000),
        ] {
            let mut pacer = pacer(10_000)?;
            pacer.set_speed(speed);
            pacer.ticks_due(&clock);
            clock.advance(Duration::from_secs(1));
            assert_eq!(pacer.ticks_due(&clock), expected, "speed {speed}");
        }
        Ok(())
    }

    #[test]
    fn test_pacer_carries_fractional_ticks() -> Result<()> {
        let clock = ManualClock::new();
        let mut pacer = pacer(1000)?;
        pacer.set_speed(SimSpeed::X1);
        pacer.ticks_due(&clock);
        let mut total = 0;
        for _ in 0..10 {
            clock.advance(Duration::from_millis(250));
            total += pacer.ticks_due(&clock);
        }
        assert_eq!(total, 2);
        Ok(())
    }

    #[test]
    fn test_pacer_paused_yields_nothing_and_drops_backlog() -> Result<()> {
        let clock = ManualClock::new();
        let mut pacer = pacer(1000)?;
        pacer.set_speed(SimSpeed::X1);
        pacer.ticks_due(&clock);
        clock.advance(Duration::from_millis(900));
        assert_eq!(pacer.ticks_due(&clock), 0);
        pacer.set_speed(SimSpeed::Paused);
        clock.advance(Duration::from_secs(60));
        assert_eq!(pacer.ticks_due(&clock), 0);
        pacer.set_speed(SimSpeed::X1);
        clock.advance(Duration::from_millis(200));
        assert_eq!(
            pacer.ticks_due(&clock),
            0,
            "paused time and old remainder must not count"
        );
        Ok(())
    }

    #[test]
    fn test_pacer_caps_ticks_per_frame_without_backlog() -> Result<()> {
        let clock = ManualClock::new();
        let mut pacer = pacer(50)?;
        pacer.set_speed(SimSpeed::X1000);
        pacer.ticks_due(&clock);
        clock.advance(Duration::from_secs(1));
        assert_eq!(pacer.ticks_due(&clock), 50);
        clock.advance(Duration::from_millis(10));
        assert_eq!(
            pacer.ticks_due(&clock),
            10,
            "dropped backlog must not spiral"
        );
        Ok(())
    }

    #[test]
    fn test_pacer_max_speed_runs_cap_every_frame() -> Result<()> {
        let clock = ManualClock::new();
        let mut pacer = pacer(25)?;
        pacer.set_speed(SimSpeed::Max);
        assert_eq!(pacer.ticks_due(&clock), 25);
        assert_eq!(pacer.ticks_due(&clock), 25);
        Ok(())
    }

    #[test]
    fn test_sim_speed_faster_slower_and_display() {
        assert_eq!(SimSpeed::Paused.faster(), SimSpeed::X1);
        assert_eq!(SimSpeed::X60.faster(), SimSpeed::X300);
        assert_eq!(SimSpeed::Max.faster(), SimSpeed::Max);
        assert_eq!(SimSpeed::X60.slower(), SimSpeed::X10);
        assert_eq!(SimSpeed::Paused.slower(), SimSpeed::Paused);
        assert_eq!(SimSpeed::X60.to_string(), "60×");
        assert_eq!(SimSpeed::Paused.to_string(), "paused");
        assert_eq!(SimSpeed::Max.to_string(), "max");
    }

    #[test]
    fn test_wall_instant_saturating_since() {
        let early = WallInstant::from_elapsed(Duration::from_secs(1));
        let late = WallInstant::from_elapsed(Duration::from_secs(3));
        assert_eq!(late.saturating_since(early), Duration::from_secs(2));
        assert_eq!(early.saturating_since(late), Duration::ZERO);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_pacer_total_ticks_match_elapsed_time(
            frames in prop::collection::vec(0_u64..50_000_000, 1..200),
        ) {
            let clock = ManualClock::new();
            let mut pacer = Pacer::new(SimDuration::from_secs(1), u32::MAX)
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
            pacer.ticks_due(&clock);
            let mut total_ticks = 0_u64;
            for nanos in &frames {
                clock.advance(Duration::from_nanos(*nanos));
                total_ticks += u64::from(pacer.ticks_due(&clock));
            }
            let total_nanos: u64 = frames.iter().sum();
            prop_assert_eq!(total_ticks, total_nanos * 60 / 1_000_000_000);
        }
    }
}
