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
/// Take-offs give way to an aircraft that has waited this long to cross.
const CROSSING_PATIENCE: SimDuration = SimDuration::from_secs(60);
/// A crossing request lapses unless renewed within this time: the aircraft
/// may have found another way.
const CROSSING_RENEWAL: SimDuration = SimDuration::from_secs(20);
/// A runway may be crossed this long after a touchdown at the earliest.
const CROSSING_AFTER_LANDING: SimDuration = SimDuration::from_secs(20);
/// After waiting this long to cross, an aircraft gets a gap in the arrivals
/// (plan M5 D128): no more approaches are cleared until it has crossed.
const CROSSING_GAP_AFTER: SimDuration = SimDuration::from_secs(180);
/// A rolling landing keeps the runway occupied this long ahead.
const ROLLING_MARGIN: SimDuration = SimDuration::from_secs(10);
/// A crossing ends at least this long before the next touchdown.
const CROSSING_BEFORE_LANDING: SimDuration = SimDuration::from_secs(30);
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
    /// A landing standing on it, unable to vacate, until this time (plan M4
    /// D90).
    blocked_until: Option<SimTime>,
    /// Since when an aircraft has waited to cross (plan M4 D77), and when
    /// it last asked.
    crossing_waits: Option<(SimTime, SimTime)>,
}

/// Whether, at `now`, an aircraft has waited long enough to cross `runway`
/// to be given a gap in the arrivals, and still asks.
fn gap_wanted(runway: &RunwayState, now: SimTime) -> bool {
    runway.crossing_waits.is_some_and(|(first, asked)| {
        now <= after(asked, CROSSING_RENEWAL) && now >= after(first, CROSSING_GAP_AFTER)
    })
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
    #[cfg(test)]
    pub(crate) fn may_take_off(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        now: SimTime,
    ) -> bool {
        self.may_line_up(key, wake, now, SimDuration::ZERO)
    }

    /// Whether an aircraft of `wake` may line up at `now` and roll after
    /// `lineup` (plan M4 D78): judged at its roll, so it may line up while
    /// the last departure still rolls.
    pub(crate) fn may_line_up(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        now: SimTime,
        lineup: SimDuration,
    ) -> bool {
        let roll = after(now, lineup);
        let runway = self.runway(key);
        let free = runway.occupied_until.is_none_or(|until| roll >= until);
        let spaced = runway
            .last_departure
            .is_none_or(|(time, leader)| roll >= after(time, departure_spacing(leader, wake)));
        let gap = after(roll, ARRIVAL_GAP);
        let booked = runway
            .timeline
            .next_after(now)
            .is_none_or(|next| next >= gap);
        let cleared = runway
            .last_landing
            .is_none_or(|(touchdown, _)| touchdown <= now || touchdown >= gap);
        // An aircraft waiting a minute to cross goes first.
        let crossers = runway.crossing_waits.is_none_or(|(since, asked)| {
            now < after(since, CROSSING_PATIENCE) || now > after(asked, CROSSING_RENEWAL)
        });
        free && spaced && booked && cleared && crossers
    }

    /// Records a take-off clearance; the roll starts after `lineup` (plan
    /// M4 D78), which occupies the runway too.
    pub(crate) fn took_off(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        now: SimTime,
        lineup: SimDuration,
    ) {
        let runway = self.runway(key);
        let roll = after(now, lineup);
        runway.last_departure = Some((roll, wake));
        runway.occupied_until = Some(after(roll, TAKEOFF_OCCUPANCY));
    }

    /// Whether the runway can be crossed during `[from, until)` (plan M4
    /// D77): not occupied, and no landing within a minute of the crossing.
    pub(crate) fn may_cross(&self, key: RunwayKey, from: SimTime, until: SimTime) -> bool {
        let Some(runway) = self
            .runways
            .get(key.airport.as_usize())
            .and_then(|runways| runways.get(key.runway))
        else {
            return true;
        };
        let free = runway.occupied_until.is_none_or(|busy| busy <= from);
        // With a gap opened, booked slots wait: only landings already
        // cleared to approach count.
        let gap = gap_wanted(runway, from);
        // No touchdown from shortly before the crossing starts until shortly
        // after it ends: the landing must have passed, the next not be on
        // short final.
        let early = from.checked_sub(CROSSING_AFTER_LANDING).unwrap_or(from);
        let late = after(until, CROSSING_BEFORE_LANDING);
        let landing = (!gap
            && runway
                .timeline
                .next_after(early)
                .is_some_and(|slot| slot < late))
            || runway
                .last_landing
                .is_some_and(|(touchdown, _)| touchdown > early && touchdown < late);
        free && !landing
    }

    /// Records that an aircraft, asking `now`, has waited to cross since
    /// `since`: after a minute, take-offs give way to it while it keeps
    /// asking.
    pub(crate) fn crossing_wanted(&mut self, key: RunwayKey, since: SimTime, now: SimTime) {
        let runway = self.runway(key);
        let first = runway
            .crossing_waits
            .filter(|(_, asked)| now <= after(*asked, CROSSING_RENEWAL))
            .map_or(since, |(earlier, _)| earlier.min(since));
        runway.crossing_waits = Some((first, now));
    }

    /// Records a crossing under way until `until`.
    pub(crate) fn crossing(&mut self, key: RunwayKey, until: SimTime) {
        let runway = self.runway(key);
        runway.occupied_until = Some(runway.occupied_until.map_or(until, |busy| busy.max(until)));
        runway.crossing_waits = None;
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

    /// Frees a booked landing slot, e.g. one the flight can no longer make.
    pub(crate) fn cancel_arrival(&mut self, key: RunwayKey, slot: SimTime) {
        self.runway(key).timeline.cancel(slot);
    }

    /// Whether, asked `now`, an aircraft touching down at `touchdown`
    /// keeps wake separation behind the last one cleared to approach, and
    /// no aircraft waiting to cross has been promised a gap.
    pub(crate) fn may_approach(
        &mut self,
        key: RunwayKey,
        wake: WakeCategory,
        approach_speed: f64,
        touchdown: SimTime,
        now: SimTime,
    ) -> bool {
        let runway = self.runway(key);
        !gap_wanted(runway, now)
            && runway.last_landing.is_none_or(|(time, leader)| {
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

    /// Whether, asked `now`, the runway is clear for a touchdown at
    /// `touchdown` (plan M4 D90): no landing standing on it unable to
    /// vacate, and no landing roll, take-off or crossing occupying it then.
    pub(crate) fn runway_clear(&self, key: RunwayKey, now: SimTime, touchdown: SimTime) -> bool {
        self.runways
            .get(key.airport.as_usize())
            .and_then(|runways| runways.get(key.runway))
            .is_none_or(|runway| {
                runway.blocked_until.is_none_or(|until| until <= now)
                    && runway.occupied_until.is_none_or(|until| until <= touchdown)
            })
    }

    /// Records a landing standing on the runway, unable to vacate, until
    /// `until`: ground control renews it while it stands there.
    pub(crate) fn blocked(&mut self, key: RunwayKey, until: SimTime) {
        self.runway(key).blocked_until = Some(until);
    }

    /// Records a touchdown: the runway is occupied by the landing roll.
    pub(crate) fn landed(&mut self, key: RunwayKey, now: SimTime) {
        self.runway(key).occupied_until = Some(after(now, RUNWAY_OCCUPANCY));
    }

    /// A landing still rolls on the runway at `now`, towards an exit
    /// further on than most: it stays occupied a little longer.
    pub(crate) fn rolling(&mut self, key: RunwayKey, now: SimTime) {
        let runway = self.runway(key);
        let until = after(now, ROLLING_MARGIN);
        runway.occupied_until = Some(runway.occupied_until.map_or(until, |busy| busy.max(until)));
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
    fn test_a_landing_needs_the_runway_clear_at_touchdown() -> Result<()> {
        let mut control = control();
        assert!(control.runway_clear(key(), at(0)?, at(60)?));
        // A landing standing on the runway, unable to vacate: reported by
        // ground control each second while it stands there.
        control.blocked(key(), at(1)?);
        assert!(!control.runway_clear(key(), at(0)?, at(60)?));
        assert!(control.runway_clear(key(), at(2)?, at(60)?));
        // The roll of the landing before is over by touchdown.
        control.landed(key(), at(0)?);
        assert!(control.runway_clear(key(), at(10)?, at(60)?));
        // A take-off rolling at touchdown.
        control.took_off(key(), WakeCategory::Medium, at(100)?, SimDuration::ZERO);
        assert!(!control.runway_clear(key(), at(90)?, at(120)?));
        assert!(control.runway_clear(key(), at(150)?, at(200)?));
        Ok(())
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
        control.took_off(key(), WakeCategory::Heavy, at(0)?, SimDuration::ZERO);
        // The runway is busy with the roll, then a medium waits 2 minutes.
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(30)?));
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(110)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(120)?));
        Ok(())
    }

    #[test]
    fn test_line_up_occupies_the_runway_before_the_roll() -> Result<()> {
        let mut control = control();
        control.took_off(
            key(),
            WakeCategory::Medium,
            at(0)?,
            SimDuration::from_secs(40),
        );
        // Lined up for 40 s, then rolling: busy until the roll is done.
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(60)?));
        assert!(!control.may_cross(key(), at(60)?, at(90)?));
        // Spacing behind it counts from its roll at 40 s.
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(95)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(100)?));
        Ok(())
    }

    #[test]
    fn test_take_offs_give_way_to_a_waiting_crossing() -> Result<()> {
        let mut control = control();
        control.crossing_wanted(key(), at(0)?, at(0)?);
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(30)?));
        // Renewed while it waits: take-offs stop after a minute.
        control.crossing_wanted(key(), at(0)?, at(55)?);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(60)?));
        control.crossing(key(), at(90)?);
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(90)?));
        // A request nobody renews lapses: the aircraft found another way.
        control.crossing_wanted(key(), at(100)?, at(170)?);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(180)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(200)?));
        Ok(())
    }

    #[test]
    fn test_next_departure_lines_up_while_the_last_one_rolls() -> Result<()> {
        let mut control = control();
        control.took_off(key(), WakeCategory::Medium, at(0)?, SimDuration::ZERO);
        let lineup = SimDuration::from_secs(30);
        // Rolling until 45 s, spacing 60 s: lining up at 30 s rolls at 60 s.
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(30)?));
        assert!(control.may_line_up(key(), WakeCategory::Medium, at(30)?, lineup));
        assert!(!control.may_line_up(key(), WakeCategory::Medium, at(20)?, lineup));
        // Not if a landing is due before its roll is done.
        control.book_arrival(key(), at(110)?, WakeCategory::Medium, 70.0);
        assert!(!control.may_line_up(key(), WakeCategory::Medium, at(30)?, lineup));
        Ok(())
    }

    #[test]
    fn test_a_landing_occupies_the_runway_until_it_vacates() -> Result<()> {
        let mut control = control();
        control.landed(key(), at(0)?);
        assert!(!control.may_cross(key(), at(30)?, at(60)?));
        // A short roll: crossable a minute after touchdown.
        assert!(control.may_cross(key(), at(70)?, at(100)?));
        // Still rolling at 80 s towards a late exit: not until it is off.
        control.rolling(key(), at(80)?);
        assert!(!control.may_cross(key(), at(85)?, at(115)?));
        assert!(control.may_cross(key(), at(95)?, at(125)?));
        Ok(())
    }

    #[test]
    fn test_a_long_wait_to_cross_opens_a_gap_in_arrivals() -> Result<()> {
        let mut control = control();
        // Landings booked every 70 s: never a gap of 90 s.
        for k in 0..20 {
            control.book_arrival(key(), at(70 * k)?, WakeCategory::Medium, 70.0);
        }
        control.crossing_wanted(key(), at(0)?, at(0)?);
        assert!(!control.may_cross(key(), at(100)?, at(140)?));
        assert!(control.may_approach(key(), WakeCategory::Medium, 70.0, at(500)?, at(100)?));
        control.approach_started(key(), WakeCategory::Medium, at(500)?);
        // Waiting three minutes and still asking: no more approaches…
        control.crossing_wanted(key(), at(0)?, at(185)?);
        assert!(!control.may_approach(key(), WakeCategory::Medium, 70.0, at(600)?, at(190)?));
        // …and once the last one cleared is down, it crosses, booked slots
        // or not.
        assert!(!control.may_cross(key(), at(470)?, at(510)?));
        control.crossing_wanted(key(), at(0)?, at(515)?);
        assert!(control.may_cross(key(), at(520)?, at(560)?));
        control.crossing(key(), at(560)?);
        // Crossed: approaches resume.
        assert!(control.may_approach(key(), WakeCategory::Medium, 70.0, at(700)?, at(560)?));
        Ok(())
    }

    #[test]
    fn test_crossings_fit_between_movements() -> Result<()> {
        let mut control = control();
        assert!(control.may_cross(key(), at(0)?, at(40)?));
        control.book_arrival(key(), at(100)?, WakeCategory::Medium, 70.0);
        // Not across a landing's approach and roll.
        assert!(!control.may_cross(key(), at(60)?, at(100)?));
        assert!(control.may_cross(key(), at(0)?, at(30)?));
        assert!(control.may_cross(key(), at(200)?, at(230)?));
        // A crossing under way keeps take-offs off the runway.
        control.crossing(key(), at(230)?);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(210)?));
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(230)?));
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
        assert!(control.may_approach(key(), WakeCategory::Heavy, 75.0, at(300)?, at(0)?));
        control.approach_started(key(), WakeCategory::Heavy, at(300)?);
        let spacing = arrival_spacing(WakeCategory::Heavy, WakeCategory::Medium, 70.0);
        let too_soon = at(300)?
            .checked_add(spacing)?
            .checked_sub(SimDuration::from_secs(1))?;
        assert!(!control.may_approach(key(), WakeCategory::Medium, 70.0, too_soon, at(0)?));
        assert!(control.may_approach(
            key(),
            WakeCategory::Medium,
            70.0,
            at(300)?.checked_add(spacing)?,
            at(0)?
        ));
        control.landed(key(), at(300)?);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(320)?));
        Ok(())
    }

    #[test]
    fn test_cancel_arrival_frees_the_runway() -> Result<()> {
        let mut control = control();
        let slot = control.book_arrival(key(), at(60)?, WakeCategory::Medium, 70.0);
        assert!(!control.may_take_off(key(), WakeCategory::Medium, at(0)?));
        control.cancel_arrival(key(), slot);
        assert!(control.may_take_off(key(), WakeCategory::Medium, at(0)?));
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
