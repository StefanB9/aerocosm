//! Runway control and arrival management (plan M3 PR 3).

mod arrival;
mod separation;
mod sequence;

pub(crate) use self::{
    arrival::plan_arrival,
    separation::{arrival_spacing, departure_spacing},
};
use self::{separation::RUNWAY_OCCUPANCY, sequence::Timeline};
use crate::{Id, SimDuration, SimTime, WakeCategory, world::Airport};

/// A take-off occupies the runway about this long.
const TAKEOFF_OCCUPANCY: SimDuration = SimDuration::from_secs(45);
/// No take-off when a landing on the same runway is due within this time.
const ARRIVAL_GAP: SimDuration = SimDuration::from_secs(90);
/// The lowest holding level, m (7,000 ft)…
const LOWEST_LEVEL: f64 = 2_133.6;
/// …and the spacing between levels, m (1,000 ft).
const LEVEL_STEP: f64 = 304.8;

/// A physical runway: index among its airport's runways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RunwayKey {
    pub(crate) airport: Id<Airport>,
    pub(crate) runway: usize,
}

/// What one runway is doing.
#[derive(Debug, Clone, Default, PartialEq)]
struct RunwayState {
    timeline: Timeline,
    last_departure: Option<(SimTime, WakeCategory)>,
    /// Predicted touchdown of the last aircraft cleared to approach.
    last_landing: Option<(SimTime, WakeCategory)>,
    occupied_until: Option<SimTime>,
}

/// Altitude of holding level `level`, m.
pub(crate) fn level_altitude(level: usize) -> f64 {
    // Levels are a handful per stack.
    #[allow(clippy::cast_precision_loss)]
    let step = level as f64;
    LOWEST_LEVEL + LEVEL_STEP * step
}

/// Runway control at the sequenced airports (D56, D57, D61).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Control {
    /// Per airport index: whether it is controlled. Dense tables, because
    /// every queued and holding aircraft looks its runway up every tick.
    sequenced: Vec<bool>,
    /// Per airport index, per runway index.
    runways: Vec<Vec<RunwayState>>,
    /// Per airport index: holding levels in use.
    stacks: Vec<Vec<bool>>,
}

fn after(time: SimTime, spacing: SimDuration) -> SimTime {
    time.checked_add(spacing).unwrap_or(time)
}

impl Control {
    /// Controls the runways of `airports`.
    pub(crate) fn new(airports: impl IntoIterator<Item = Id<Airport>>) -> Self {
        Self {
            sequenced: {
                let mut sequenced = Vec::new();
                for airport in airports {
                    let index = airport.as_usize();
                    if sequenced.len() <= index {
                        sequenced.resize(index + 1, false);
                    }
                    sequenced[index] = true;
                }
                sequenced
            },
            ..Self::default()
        }
    }

    /// Whether `airport`'s runways are controlled.
    pub(crate) fn is_sequenced(&self, airport: Id<Airport>) -> bool {
        self.sequenced
            .get(airport.as_usize())
            .copied()
            .unwrap_or(false)
    }

    fn runway(&mut self, key: RunwayKey) -> &mut RunwayState {
        let airport = key.airport.as_usize();
        if self.runways.len() <= airport {
            self.runways.resize_with(airport + 1, Vec::new);
        }
        let runways = &mut self.runways[airport];
        if runways.len() <= key.runway {
            runways.resize_with(key.runway + 1, RunwayState::default);
        }
        &mut runways[key.runway]
    }

    /// Whether an aircraft of `wake` may take off at `now`: the runway is
    /// free, the last departure is far enough ahead and no landing is due
    /// within the arrival gap.
    pub(crate) fn may_take_off(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        now: SimTime,
    ) -> bool {
        let runway = self.runway(key);
        let free = runway.occupied_until.is_none_or(|until| now >= until);
        let spaced = runway
            .last_departure
            .is_none_or(|(time, leader)| now >= after(time, departure_spacing(leader, wake)));
        let gap = after(now, ARRIVAL_GAP);
        let booked = runway
            .timeline
            .next_after(now)
            .is_none_or(|next| next >= gap);
        let cleared = runway
            .last_landing
            .is_none_or(|(touchdown, _)| touchdown <= now || touchdown >= gap);
        free && spaced && booked && cleared
    }

    /// Records a take-off clearance.
    pub(crate) fn took_off(&mut self, key: RunwayKey, wake: WakeCategory, now: SimTime) {
        let runway = self.runway(key);
        runway.last_departure = Some((now, wake));
        runway.occupied_until = Some(after(now, TAKEOFF_OCCUPANCY));
    }

    /// Books a landing slot at or after `eta`.
    pub(crate) fn book_arrival(
        &mut self,
        key: RunwayKey,
        eta: SimTime,
        wake: WakeCategory,
        approach_speed: f64,
    ) -> SimTime {
        self.runway(key).timeline.book(eta, wake, approach_speed)
    }

    /// Whether an aircraft touching down at `touchdown` keeps wake
    /// separation behind the last one cleared to approach.
    pub(crate) fn may_approach(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        approach_speed: f64,
        touchdown: SimTime,
    ) -> bool {
        self.runway(key).last_landing.is_none_or(|(time, leader)| {
            touchdown >= after(time, arrival_spacing(leader, wake, approach_speed))
        })
    }

    /// Records an approach clearance with its predicted touchdown.
    pub(crate) fn approach_started(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        touchdown: SimTime,
    ) {
        self.runway(key).last_landing = Some((touchdown, wake));
    }

    /// Records a touchdown: the runway is occupied by the landing roll.
    pub(crate) fn landed(&mut self, key: RunwayKey, now: SimTime) {
        self.runway(key).occupied_until = Some(after(now, RUNWAY_OCCUPANCY));
    }

    /// Forgets booked landings before `time`.
    pub(crate) fn prune(&mut self, time: SimTime) {
        for runway in self.runways.iter_mut().flatten() {
            runway.timeline.prune(time);
        }
    }

    /// The lowest free holding level over `airport`.
    pub(crate) fn take_level(&mut self, airport: Id<Airport>) -> usize {
        let index = airport.as_usize();
        if self.stacks.len() <= index {
            self.stacks.resize_with(index + 1, Vec::new);
        }
        let stack = &mut self.stacks[index];
        let level = stack.iter().position(|used| !used).unwrap_or(stack.len());
        if level == stack.len() {
            stack.push(true);
        } else {
            stack[level] = true;
        }
        level
    }

    /// Frees a holding level.
    pub(crate) fn release_level(&mut self, airport: Id<Airport>, level: usize) {
        if let Some(used) = self
            .stacks
            .get_mut(airport.as_usize())
            .and_then(|stack| stack.get_mut(level))
        {
            *used = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn at(seconds: i32) -> Result<SimTime> {
        "2026-07-08T10:00:00Z"
            .parse::<SimTime>()?
            .checked_add(SimDuration::from_secs(seconds))
    }

    fn key() -> RunwayKey {
        RunwayKey {
            airport: Id::new(3),
            runway: 1,
        }
    }

    fn control() -> Control {
        Control::new([Id::new(3)])
    }

    #[test]
    fn test_only_listed_airports_are_sequenced() {
        let control = control();
        assert!(control.is_sequenced(Id::new(3)));
        assert!(!control.is_sequenced(Id::new(4)));
    }

    #[test]
    fn test_departures_keep_wake_separation_and_wait_for_occupancy() -> Result<()> {
        let mut control = control();
        assert!(control.may_take_off(key(), WakeCategory::Heavy, at(0)?));
        control.took_off(key(), WakeCategory::Heavy, at(0)?);
        // The runway is busy with the roll, then a medium waits 2 minutes.
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(30)?));
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(110)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(120)?));
        Ok(())
    }

    #[test]
    fn test_departures_leave_a_gap_before_the_next_landing() -> Result<()> {
        let mut control = control();
        control.book_arrival(key(), at(60)?, WakeCategory::Medium, 70.0);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(0)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(200)?));
        Ok(())
    }

    #[test]
    fn test_approach_gate_keeps_landings_apart() -> Result<()> {
        let mut control = control();
        assert!(control.may_approach(key(), WakeCategory::Heavy, 75.0, at(300)?));
        control.approach_started(key(), WakeCategory::Heavy, at(300)?);
        let spacing = arrival_spacing(WakeCategory::Heavy, WakeCategory::Medium, 70.0);
        let too_soon = at(300)?
            .checked_add(spacing)?
            .checked_sub(SimDuration::from_secs(1))?;
        assert!(!control.may_approach(key(), WakeCategory::Medium, 70.0, too_soon));
        assert!(control.may_approach(
            key(),
            WakeCategory::Medium,
            70.0,
            at(300)?.checked_add(spacing)?
        ));
        control.landed(key(), at(300)?);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(320)?));
        Ok(())
    }

    #[test]
    fn test_prune_forgets_past_landings() -> Result<()> {
        let mut control = control();
        control.book_arrival(key(), at(60)?, WakeCategory::Medium, 70.0);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(0)?));
        control.prune(at(120)?);
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(0)?));
        Ok(())
    }

    #[test]
    fn test_stack_levels_are_shared_out_lowest_first() {
        let mut control = control();
        let first = control.take_level(Id::new(3));
        let second = control.take_level(Id::new(3));
        assert_eq!((first, second), (0, 1));
        control.release_level(Id::new(3), first);
        assert_eq!(control.take_level(Id::new(3)), 0);
        assert!((level_altitude(0) - 2_133.6).abs() < 0.1, "7,000 ft");
        assert!((level_altitude(2) - 2_743.2).abs() < 0.1, "9,000 ft");
    }
}
