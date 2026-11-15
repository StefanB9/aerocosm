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

/// Receives the start of each part of a tick.
pub(crate) trait Timer {
    /// From now on the tick is in `part`.
    fn enter(&mut self, part: Part);
}

/// Times nothing.
pub(crate) struct NoTimer;

impl Timer for NoTimer {
    #[inline]
    fn enter(&mut self, _part: Part) {}
}

/// Attributes the time between readings of `clock` to the part entered at
/// the earlier reading.
pub(crate) struct ClockTimer<'a, C: WallClock> {
    clock: &'a C,
    current: Option<(Part, WallInstant)>,
    times: PhaseTimes,
}

impl<'a, C: WallClock> ClockTimer<'a, C> {
    pub(crate) const fn new(clock: &'a C) -> Self {
        Self {
            clock,
            current: None,
            times: PhaseTimes {
                flow_control: Duration::ZERO,
                ground_control: Duration::ZERO,
                taxiing: Duration::ZERO,
                flying: Duration::ZERO,
                parked: Duration::ZERO,
            },
        }
    }

    /// The times so far, the last part ending now.
    pub(crate) fn finish(mut self) -> PhaseTimes {
        self.close(self.clock.now());
        self.times
    }

    fn close(&mut self, now: WallInstant) {
        if let Some((part, since)) = self.current.take() {
            let spent = now.saturating_since(since);
            let slot = match part {
                Part::FlowControl => &mut self.times.flow_control,
                Part::GroundControl => &mut self.times.ground_control,
                Part::Taxiing => &mut self.times.taxiing,
                Part::Flying => &mut self.times.flying,
                Part::Parked => &mut self.times.parked,
            };
            *slot += spent;
        }
    }
}

impl<C: WallClock> Timer for ClockTimer<'_, C> {
    fn enter(&mut self, part: Part) {
        // Consecutive tails in the same part share one reading.
        if self.current.is_some_and(|(current, _)| current == part) {
            return;
        }
        let now = self.clock.now();
        self.close(now);
        self.current = Some((part, now));
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
    fn test_clock_timer_attributes_time_to_the_part_entered() {
        let clock = Stepping(Cell::new(0));
        let mut timer = ClockTimer::new(&clock);
        timer.enter(Part::GroundControl); // 0
        timer.enter(Part::Flying); // 1
        timer.enter(Part::Flying); // no reading
        timer.enter(Part::Parked); // 2
        timer.enter(Part::Flying); // 3
        let spent = timer.finish(); // 4
        assert_eq!(spent.ground_control, Duration::from_millis(1));
        assert_eq!(spent.flying, Duration::from_millis(2));
        assert_eq!(spent.parked, Duration::from_millis(1));
        assert_eq!(spent.total(), Duration::from_millis(4));
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
        let mut timer = ClockTimer::new(&clock);
        timer.enter(Part::FlowControl);
        NoTimer.enter(Part::Taxiing);
        assert_eq!(timer.finish().flow_control, Duration::from_millis(1));
    }
}
