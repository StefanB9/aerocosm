//! Placing the weekly template in real time (D43): a tail's week starts at
//! its base's local Monday 00:00, and each template time goes to UTC through
//! the base's zone, so daylight-saving weeks come out right.

use anyhow::{Context, Result};

use crate::{
    CoreError, SimDuration, SimTime,
    time::{LocalDate, Zone},
};

/// Minutes in a day.
const DAY_MINUTES: i64 = 24 * 60;

/// The local Monday of the week containing `time` in `zone`.
pub(crate) fn week_of(zone: &Zone, time: SimTime) -> Result<LocalDate> {
    let local = zone.local(time);
    let back = i32::try_from(local.weekday().index()).context("weekday index")?;
    local.date().add_days(-back)
}

/// The instant of template time `offset` (since local Monday 00:00) in the
/// week starting on `monday`.
pub(crate) fn place(zone: &Zone, monday: LocalDate, offset: SimDuration) -> Result<SimTime> {
    let minutes = offset.as_millis() / 60_000;
    if minutes < 0 {
        return Err(CoreError::ScheduleFailed {
            reason: format!("negative template time {offset}"),
        }
        .into());
    }
    let days = i32::try_from(minutes / DAY_MINUTES).context("template day")?;
    let minute = u16::try_from(minutes % DAY_MINUTES).context("template minute")?;
    zone.to_utc(monday.add_days(days)?, minute)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn berlin() -> Result<Zone> {
        Zone::new("Europe/Berlin")
    }

    #[test]
    fn test_week_of_finds_the_local_monday() -> Result<()> {
        let zone = berlin()?;
        assert_eq!(
            week_of(&zone, "2026-07-08T12:00:00Z".parse()?)?,
            LocalDate::new(2026, 7, 6)?
        );
        // Monday 00:30 local is still Sunday in UTC.
        assert_eq!(
            week_of(&zone, "2026-07-05T22:30:00Z".parse()?)?,
            LocalDate::new(2026, 7, 6)?
        );
        assert_eq!(
            week_of(&zone, "2026-07-05T21:30:00Z".parse()?)?,
            LocalDate::new(2026, 6, 29)?
        );
        Ok(())
    }

    #[test]
    fn test_place_converts_template_minutes_to_utc() -> Result<()> {
        let zone = berlin()?;
        let monday = LocalDate::new(2026, 7, 6)?;
        assert_eq!(
            place(&zone, monday, SimDuration::ZERO)?,
            "2026-07-05T22:00:00Z".parse()?
        );
        let sunday_late = SimDuration::from_minutes(6 * 1440 + 23 * 60 + 15);
        assert_eq!(
            place(&zone, monday, sunday_late)?,
            "2026-07-12T21:15:00Z".parse()?
        );
        // Past the week: the return of a Sunday-night trip lands next Monday.
        let next_monday = SimDuration::from_minutes(7 * 1440 + 60);
        assert_eq!(
            place(&zone, monday, next_monday)?,
            "2026-07-12T23:00:00Z".parse()?
        );
        Ok(())
    }

    #[test]
    fn test_place_follows_daylight_saving() -> Result<()> {
        let zone = berlin()?;
        let monday = LocalDate::new(2026, 3, 23)?;
        // Monday 08:00 is CET, the next Monday 08:00 CEST.
        let eight = SimDuration::from_minutes(8 * 60);
        assert_eq!(
            place(&zone, monday, eight)?,
            "2026-03-23T07:00:00Z".parse()?
        );
        assert_eq!(
            place(&zone, monday.add_days(7)?, eight)?,
            "2026-03-30T06:00:00Z".parse()?
        );
        // 02:30 on the switch night does not exist and moves to 03:30 CEST.
        let skipped = SimDuration::from_minutes(6 * 1440 + 150);
        assert_eq!(
            place(&zone, monday, skipped)?,
            "2026-03-29T01:30:00Z".parse()?
        );
        Ok(())
    }

    #[test]
    fn test_place_rejects_negative_template_times() -> Result<()> {
        let monday = LocalDate::new(2026, 7, 6)?;
        assert!(place(&berlin()?, monday, SimDuration::from_minutes(-1)).is_err());
        Ok(())
    }
}
