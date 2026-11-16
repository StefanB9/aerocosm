//! Where a tick's wall time goes (plan M5 D136), for benchmarks only: the
//! fleet's step marks each part of the tick on a timer, which costs nothing
//! when the timer reads no clock. Timing never changes what happens.

use std::{
    ops::{Add, AddAssign},
    time::Duration,
};

use crate::{WallClock, WallInstant};

/// Wall time spent in each part of one or more ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhaseTimes {
    /// Flow control's housekeeping (forgetting old landing slots).
    pub flow_control: Duration,
    /// The ground controller: survey, deadlocks, clearances.
    pub ground_control: Duration,
    /// Aircraft pushing back or taxiing under ground control.
    pub taxiing: Duration,
    /// Aircraft flying, and taxiing where no layout is mapped.
    pub flying: Duration,
    /// Aircraft parked or boarding, including towing and pushback approval.
    pub parked: Duration,
}

impl PhaseTimes {
    /// The sum of all parts.
    pub fn total(&self) -> Duration {
        self.flow_control + self.ground_control + self.taxiing + self.flying + self.parked
    }
}

impl Add for PhaseTimes {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            flow_control: self.flow_control + other.flow_control,
            ground_control: self.ground_control + other.ground_control,
            taxiing: self.taxiing + other.taxiing,
            flying: self.flying + other.flying,
            parked: self.parked + other.parked,
        }
    }
}

impl AddAssign for PhaseTimes {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// A part of the tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    FlowControl,
    GroundControl,
    Taxiing,
    Flying,
    Parked,
}

/// One tail in this many is timed per tick, the others not at all: reading
/// the clock at every tail would cost a sizeable share of the tick itself.
/// The timed tails rotate from tick to tick, so over many ticks every tail is
/// timed equally often.
pub(crate) const SAMPLE: usize = 16;

/// Pairs of readings that measure what a reading itself costs.
const CALIBRATION: usize = 16;

/// Receives the parts of a tick.
pub(crate) trait Timer {
    /// From now on the tick is in `part` (a part of the whole fleet).
    fn enter(&mut self, part: Part);

    /// Runs `step`, tail `index`'s work in `part`.
    fn tail<R>(&mut self, index: usize, part: Part, step: impl FnOnce() -> R) -> R;
}

/// Times nothing.
pub(crate) struct NoTimer;

impl Timer for NoTimer {
    #[inline]
    fn enter(&mut self, _part: Part) {}

    #[inline]
    fn tail<R>(&mut self, _index: usize, _part: Part, step: impl FnOnce() -> R) -> R {
        step()
    }
}

/// Times the parts of the fleet exactly and every `SAMPLE`th tail, scaled
/// up, with `clock`.
pub(crate) struct ClockTimer<'a, C: WallClock> {
    clock: &'a C,
    /// The tails timed this tick: those whose index has this remainder.
    offset: usize,
    /// What one reading of the clock adds to a timed tail: the cheapest of
    /// a few back-to-back readings. Scaled up with the sample, it would
    /// otherwise make cheap tails look expensive.
    overhead: Duration,
    current: Option<(Part, WallInstant)>,
    times: PhaseTimes,
}

impl<'a, C: WallClock> ClockTimer<'a, C> {
    /// A timer for tick number `tick`.
    pub(crate) fn new(clock: &'a C, tick: u64) -> Self {
        let overhead = (0..CALIBRATION)
            .map(|_| {
                let start = clock.now();
                clock.now().saturating_since(start)
            })
            .min()
            .unwrap_or_default();
        Self {
            clock,
            offset: usize::try_from(tick % SAMPLE as u64).unwrap_or(0),
            overhead,
            current: None,
            times: PhaseTimes::default(),
        }
    }

    /// The times so far, the last part ending now.
    pub(crate) fn finish(mut self) -> PhaseTimes {
        self.close();
        self.times
    }

    fn slot(&mut self, part: Part) -> &mut Duration {
        match part {
            Part::FlowControl => &mut self.times.flow_control,
            Part::GroundControl => &mut self.times.ground_control,
            Part::Taxiing => &mut self.times.taxiing,
            Part::Flying => &mut self.times.flying,
            Part::Parked => &mut self.times.parked,
        }
    }

    /// Ends the current part of the fleet, if any.
    fn close(&mut self) {
        if let Some((part, since)) = self.current.take() {
            let spent = self.clock.now().saturating_since(since);
            *self.slot(part) += spent;
        }
    }
}

impl<C: WallClock> Timer for ClockTimer<'_, C> {
    fn enter(&mut self, part: Part) {
        self.close();
        self.current = Some((part, self.clock.now()));
    }

    fn tail<R>(&mut self, index: usize, part: Part, step: impl FnOnce() -> R) -> R {
        self.close();
        if index % SAMPLE != self.offset {
            return step();
        }
        let start = self.clock.now();
        let value = step();
        let spent = self
            .clock
            .now()
            .saturating_since(start)
            .saturating_sub(self.overhead);
        *self.slot(part) += spent * u32::try_from(SAMPLE).unwrap_or(1);
        value
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A clock that advances a millisecond on every reading.
    struct Stepping(Cell<u64>);

    impl WallClock for Stepping {
        fn now(&self) -> WallInstant {
            let ms = self.0.get();
            self.0.set(ms + 1);
            WallInstant::from_elapsed(Duration::from_millis(ms))
        }
    }

    #[test]
    fn test_clock_timer_times_parts_and_every_sampled_tail() {
        let clock = Stepping(Cell::new(0));
        // Tick 3: tails 3, 19, 35, … are timed. Calibration reads 2 × 16
        // times and finds that a reading costs 1 ms.
        let mut timer = ClockTimer::new(&clock, 3);
        timer.enter(Part::FlowControl); // reads 32
        timer.enter(Part::GroundControl); // reads 33 (end), 34 (start)
        // Each tail's work takes 5 ms of the clock.
        let mut ran = 0;
        for index in 0..40 {
            let part = if index == 19 {
                Part::Parked
            } else {
                Part::Flying
            };
            ran += timer.tail(index, part, || {
                clock.0.set(clock.0.get() + 5);
                1
            });
        }
        let spent = timer.finish();
        assert_eq!(ran, 40, "every tail runs");
        assert_eq!(spent.flow_control, Duration::from_millis(1));
        // Closed by the first tail, reading 35.
        assert_eq!(spent.ground_control, Duration::from_millis(1));
        // A timed tail spans its work and one reading, less the reading's
        // cost, and stands for 16 tails.
        assert_eq!(spent.flying, Duration::from_millis(2 * 5 * 16));
        assert_eq!(spent.parked, Duration::from_millis(5 * 16));
    }

    #[test]
    fn test_phase_times_add_up() {
        let one = PhaseTimes {
            taxiing: Duration::from_millis(2),
            ..PhaseTimes::default()
        };
        let mut sum = one + one;
        sum += one;
        assert_eq!(sum.taxiing, Duration::from_millis(6));
        assert_eq!(sum.total(), Duration::from_millis(6));
        let clock = Stepping(Cell::new(0));
        let mut timer = ClockTimer::new(&clock, 0);
        timer.enter(Part::FlowControl);
        NoTimer.enter(Part::Taxiing);
        assert_eq!(NoTimer.tail(0, Part::Flying, || 7), 7);
        assert_eq!(timer.finish().flow_control, Duration::from_millis(1));
    }
}
