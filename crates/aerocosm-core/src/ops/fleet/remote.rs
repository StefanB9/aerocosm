//! Remote parking (plan M5 D125, D127): an aircraft with a long ground time
//! is towed off its stand to remote parking, which the ground layouts do not
//! map, and towed back to a free stand before it departs. Mapped stands then
//! hold the aircraft that board or turn round, not the night's fleet. Where
//! no stand comes free in half an hour, it boards remote and joins the
//! taxiways at a waiting spot.

use super::Operations;
use crate::{
    Id, SimDuration, SimTime,
    ground::Node,
    scenario::Scenario,
    world::{Airport, codes::AirlineIcao},
};

/// Ground times from reaching the stand to off-block of at least this long
/// are spent in remote parking.
const LONG_STAY: SimDuration = SimDuration::from_minutes(120);
/// The aircraft is towed off this long after reaching its stand (deplaning,
/// cleaning).
const TOW_OFF_AFTER: SimDuration = SimDuration::from_minutes(30);
/// It is towed back this long before off-block (boarding starts 30 minutes
/// before it).
const TOW_BACK: SimDuration = SimDuration::from_minutes(60);
/// Without a free stand it asks again this often…
const RETRY: SimDuration = SimDuration::from_minutes(1);
/// …and boards remote after waiting this long, as an arrival at a waiting
/// spot deplanes remote (D127).
const REMOTE_AFTER: SimDuration = SimDuration::from_minutes(30);

/// Where a tail on the ground at a mapped airport is parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Parking {
    /// On its stand (or not on the ground at a mapped airport).
    #[default]
    Stand,
    /// On its stand until it is towed off at this time.
    TowOff(SimTime),
    /// In remote parking; since when it waits for a free stand, if it does.
    Remote(Option<SimTime>),
    /// Boarded remote: it joins the taxiways at this waiting spot.
    Spot(Id<Node>),
}

/// When an aircraft that reached its stand at `arrived` and leaves it at
/// `off_block` is towed off, if its ground time is long enough.
pub(super) fn tow_off_time(arrived: SimTime, off_block: SimTime) -> Option<SimTime> {
    (off_block.duration_since(arrived) >= LONG_STAY)
        .then(|| arrived.checked_add(TOW_OFF_AFTER).ok())
        .flatten()
}

/// Whether an aircraft that leaves at `off_block` starts the simulation at
/// `start` in remote parking.
pub(super) fn starts_remote(start: SimTime, off_block: SimTime) -> bool {
    off_block.duration_since(start) > TOW_BACK
}

/// When an aircraft leaving at `off_block` is towed back to a stand.
fn tow_back_time(off_block: SimTime) -> SimTime {
    off_block.checked_sub(TOW_BACK).unwrap_or(off_block)
}

/// The first time anything can happen to an aircraft parked as `parking`
/// that boards from `boarding_from` for `off_block` (plan M5 D136): until
/// then the fleet's tick passes it by. A time already past means every tick.
pub(super) fn next_look(parking: Parking, boarding_from: SimTime, off_block: SimTime) -> SimTime {
    match parking {
        Parking::Stand | Parking::Spot(_) => boarding_from,
        Parking::TowOff(at) => at.min(boarding_from),
        Parking::Remote(_) => tow_back_time(off_block).min(boarding_from),
    }
}

impl Operations {
    /// After tail `index` reached its stand at `now`: tow-off planned if its
    /// next off-block is far enough away.
    pub(super) fn plan_parking(&mut self, index: usize, now: SimTime) {
        self.parking[index] = match (
            self.parked[index],
            tow_off_time(now, self.tails[index].off_block),
        ) {
            (Some(_), Some(at)) => Parking::TowOff(at),
            // Deplaned remote at a waiting spot.
            (None, _) if matches!(self.parking[index], Parking::Remote(_)) => Parking::Remote(None),
            _ => Parking::Stand,
        };
    }

    /// Tows tail `index`, parked at `airport` before boarding, off its stand
    /// or back to one when due. Returns whether boarding must wait because
    /// no stand is free yet.
    pub(super) fn tend_parking(
        &mut self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        airline: AirlineIcao,
        now: SimTime,
    ) -> bool {
        match self.parking[index] {
            Parking::Stand | Parking::Spot(_) => false,
            Parking::TowOff(at) => {
                if now >= at {
                    self.release_stand(index);
                    self.parking[index] = Parking::Remote(None);
                }
                false
            }
            Parking::Remote(since) => {
                let off_block = self.tails[index].off_block;
                if now < tow_back_time(off_block) {
                    return false;
                }
                let Some(near) = scenario.world().airport(airport).map(Airport::location) else {
                    self.parking[index] = Parking::Stand;
                    return false;
                };
                if self
                    .find_stand(scenario, index, airport, airline, (near, None), false)
                    .is_some()
                {
                    self.parking[index] = Parking::Stand;
                    return false;
                }
                let since = since.unwrap_or_else(|| {
                    self.stats.waited_for_stand(airport);
                    now
                });
                // After half an hour it boards remote, once a spot is free
                // to join the taxiways at (arrivals leave theirs within
                // half an hour).
                if now.duration_since(since) >= REMOTE_AFTER
                    && let Some(spot) = self.boarding_spot(scenario, airport)
                {
                    self.parking[index] = Parking::Spot(spot);
                    return false;
                }
                self.parking[index] = Parking::Remote(Some(since));
                // Boarding waits for a stand: the departure slips.
                if now >= self.tails[index].boarding_from {
                    let tail = &mut self.tails[index];
                    tail.boarding_from = now.checked_add(RETRY).unwrap_or(now);
                    tail.off_block = tail.off_block.max(
                        tail.boarding_from
                            .checked_add(super::BOARDING)
                            .unwrap_or(now),
                    );
                    return true;
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn at(minutes: i32) -> Result<SimTime> {
        SimTime::UNIX_EPOCH.checked_add(SimDuration::from_minutes(minutes))
    }

    #[test]
    fn test_long_ground_times_are_towed_off_after_deplaning() -> Result<()> {
        // Two hours or more on the ground: off after 30 minutes.
        assert_eq!(tow_off_time(at(0)?, at(120)?), Some(at(30)?));
        assert_eq!(tow_off_time(at(0)?, at(600)?), Some(at(30)?));
        // A turnaround stays on its stand.
        assert_eq!(tow_off_time(at(0)?, at(119)?), None);
        assert_eq!(tow_back_time(at(600)?), at(540)?);
        Ok(())
    }

    #[test]
    fn test_parked_aircraft_are_looked_at_when_something_is_due() -> Result<()> {
        // On a stand or a waiting spot: when boarding starts.
        assert_eq!(next_look(Parking::Stand, at(300)?, at(340)?), at(300)?);
        assert_eq!(
            next_look(Parking::Spot(crate::Id::new(3)), at(300)?, at(340)?),
            at(300)?
        );
        // Due to be towed off: then, or at boarding if that is earlier.
        assert_eq!(
            next_look(Parking::TowOff(at(30)?), at(300)?, at(340)?),
            at(30)?
        );
        assert_eq!(
            next_look(Parking::TowOff(at(30)?), at(20)?, at(60)?),
            at(20)?
        );
        // Remote: when it is towed back, an hour before off-block.
        assert_eq!(
            next_look(Parking::Remote(None), at(300)?, at(340)?),
            at(280)?
        );
        assert_eq!(
            next_look(Parking::Remote(Some(at(1)?)), at(250)?, at(340)?),
            at(250)?
        );
        Ok(())
    }

    #[test]
    fn test_the_start_puts_late_departures_in_remote_parking() -> Result<()> {
        assert!(starts_remote(at(0)?, at(61)?));
        assert!(!starts_remote(at(0)?, at(60)?));
        assert!(!starts_remote(at(0)?, at(10)?));
        Ok(())
    }
}
