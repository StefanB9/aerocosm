//! Local time at airports: IANA time zones from the database bundled with
//! `jiff` (plan D40).
//!
//! The database is compiled in, so conversions are identical on every platform
//! and in wasm, regardless of the host's zoneinfo files.

use std::fmt;

use anyhow::{Context, Result};
use jiff::{Timestamp, civil, tz::TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{CoreError, SimTime};

const MINUTES_PER_DAY: u16 = 24 * 60;

/// Day of the week, Monday first (ISO 8601).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    /// All days, Monday first.
    pub const ALL: [Self; 7] = [
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
        Self::Sunday,
    ];

    /// Position in the week, Monday = 0.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The following day.
    #[must_use]
    pub const fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % 7]
    }

    const fn from_jiff(weekday: civil::Weekday) -> Self {
        match weekday {
            civil::Weekday::Monday => Self::Monday,
            civil::Weekday::Tuesday => Self::Tuesday,
            civil::Weekday::Wednesday => Self::Wednesday,
            civil::Weekday::Thursday => Self::Thursday,
            civil::Weekday::Friday => Self::Friday,
            civil::Weekday::Saturday => Self::Saturday,
            civil::Weekday::Sunday => Self::Sunday,
        }
    }
}

/// A validated calendar date in some local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalDate(civil::Date);

impl LocalDate {
    /// Validates a calendar date.
    pub fn new(year: i16, month: i8, day: i8) -> Result<Self> {
        civil::Date::new(year, month, day).map(Self).map_err(|_| {
            CoreError::InvalidDateTime {
                reason: "day out of range for month",
            }
            .into()
        })
    }

    /// Year.
    pub fn year(self) -> i16 {
        self.0.year()
    }

    /// Month, `1..=12`.
    pub fn month(self) -> i8 {
        self.0.month()
    }

    /// Day of month.
    pub fn day(self) -> i8 {
        self.0.day()
    }

    /// The following day.
    pub fn next(self) -> Result<Self> {
        self.0.tomorrow().map(Self).context("date out of range")
    }

    /// Day of the week.
    pub fn weekday(self) -> Weekday {
        Weekday::from_jiff(self.0.weekday())
    }
}

/// A local wall-clock reading: date, weekday and minute of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    date: LocalDate,
    minute_of_day: u16,
}

impl LocalTime {
    /// Local calendar date.
    pub const fn date(self) -> LocalDate {
        self.date
    }

    /// Day of the week.
    pub fn weekday(self) -> Weekday {
        self.date.weekday()
    }

    /// Minutes since local midnight, `0..1440`.
    pub const fn minute_of_day(self) -> u16 {
        self.minute_of_day
    }
}

/// An IANA time zone such as `Europe/Berlin`.
#[derive(Clone)]
pub struct Zone {
    name: String,
    zone: TimeZone,
}

impl Zone {
    /// Looks up a zone in the bundled IANA database.
    pub fn new(name: &str) -> Result<Self> {
        let zone = TimeZone::get(name).map_err(|_| CoreError::UnknownTimeZone {
            name: name.to_owned(),
        })?;
        Ok(Self {
            name: name.to_owned(),
            zone,
        })
    }

    /// The IANA name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The local date and time at the instant `time`.
    pub fn local(&self, time: SimTime) -> LocalTime {
        // SAFETY: `SimTime` is limited to years -9999..=9999, which jiff covers
        // entirely, so the fallback is unreachable.
        let timestamp = Timestamp::from_millisecond(time.as_millis()).unwrap_or_default();
        let zoned = timestamp.to_zoned(self.zone.clone());
        let clock = zoned.time();
        #[allow(clippy::cast_sign_loss)]
        let minute_of_day = clock.hour() as u16 * 60 + clock.minute() as u16;
        LocalTime {
            date: LocalDate(zoned.date()),
            minute_of_day,
        }
    }

    /// The instant at local `minute_of_day` on `date`. A time skipped by a
    /// daylight-saving change is moved forward by the change; a repeated time
    /// takes its first occurrence.
    pub fn to_utc(&self, date: LocalDate, minute_of_day: u16) -> Result<SimTime> {
        if minute_of_day >= MINUTES_PER_DAY {
            return Err(CoreError::InvalidLocalTime {
                minute: minute_of_day,
            }
            .into());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let (hour, minute) = ((minute_of_day / 60) as i8, (minute_of_day % 60) as i8);
        let local = date.0.at(hour, minute, 0, 0);
        let zoned = self
            .zone
            .to_ambiguous_zoned(local)
            .compatible()
            .with_context(|| format!("cannot place {local} in {}", self.name))?;
        SimTime::from_millis(zoned.timestamp().as_millisecond())
    }
}

impl PartialEq for Zone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Zone {}

impl fmt::Debug for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Zone({})", self.name)
    }
}

impl Serialize for Zone {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name)
    }
}

impl<'de> Deserialize<'de> for Zone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <&str>::deserialize(deserializer)?;
        Self::new(name).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::{CoreError, SimTime};

    fn at(text: &str) -> Result<SimTime> {
        text.parse()
    }

    #[test]
    fn test_zone_rejects_unknown_names() -> Result<()> {
        let error = Zone::new("Europe/Atlantis")
            .err()
            .context("must be rejected")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnknownTimeZone {
                name: "Europe/Atlantis".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn test_berlin_winter_and_summer_offsets() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        let winter = berlin.local(at("2026-01-15T12:00:00Z")?);
        assert_eq!(winter.minute_of_day(), 13 * 60);
        assert_eq!(winter.weekday(), Weekday::Thursday);
        let summer = berlin.local(at("2026-07-15T12:00:00Z")?);
        assert_eq!(summer.minute_of_day(), 14 * 60);
        Ok(())
    }

    #[test]
    fn test_local_date_rolls_over_at_local_midnight() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        let local = berlin.local(at("2026-11-05T23:30:00Z")?);
        assert_eq!(local.minute_of_day(), 30);
        assert_eq!(local.weekday(), Weekday::Friday);
        assert_eq!(
            (
                local.date().year(),
                local.date().month(),
                local.date().day()
            ),
            (2026, 11, 6)
        );
        Ok(())
    }

    #[test]
    fn test_2026_daylight_saving_changes_in_berlin() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        // Clocks go forward on 29 March 2026 and back on 25 October 2026 at
        // 01:00 UTC.
        assert_eq!(
            berlin.local(at("2026-03-29T00:59:00Z")?).minute_of_day(),
            60 + 59
        );
        assert_eq!(
            berlin.local(at("2026-03-29T01:00:00Z")?).minute_of_day(),
            3 * 60
        );
        assert_eq!(
            berlin.local(at("2026-10-25T00:59:00Z")?).minute_of_day(),
            2 * 60 + 59
        );
        assert_eq!(
            berlin.local(at("2026-10-25T01:00:00Z")?).minute_of_day(),
            2 * 60
        );
        Ok(())
    }

    #[test]
    fn test_other_zones() -> Result<()> {
        let noon = at("2026-07-15T12:00:00Z")?;
        assert_eq!(
            Zone::new("Europe/London")?.local(noon).minute_of_day(),
            13 * 60
        );
        assert_eq!(
            Zone::new("Europe/Istanbul")?.local(noon).minute_of_day(),
            15 * 60
        );
        assert_eq!(
            Zone::new("Europe/Helsinki")?.local(noon).minute_of_day(),
            15 * 60
        );
        Ok(())
    }

    #[test]
    fn test_to_utc_known_value() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        let date = LocalDate::new(2026, 11, 5)?;
        assert_eq!(berlin.to_utc(date, 6 * 60)?, at("2026-11-05T05:00:00Z")?);
        Ok(())
    }

    #[test]
    fn test_to_utc_inside_spring_gap_moves_forward() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        // 02:30 does not exist on 29 March 2026; it is taken as 03:30 CEST.
        let date = LocalDate::new(2026, 3, 29)?;
        assert_eq!(
            berlin.to_utc(date, 2 * 60 + 30)?,
            at("2026-03-29T01:30:00Z")?
        );
        Ok(())
    }

    #[test]
    fn test_to_utc_rejects_minute_out_of_range() -> Result<()> {
        let berlin = Zone::new("Europe/Berlin")?;
        let error = berlin
            .to_utc(LocalDate::new(2026, 1, 1)?, 1440)
            .err()
            .context("must fail")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::InvalidLocalTime { minute: 1440 })
        );
        Ok(())
    }

    #[test]
    fn test_zone_serde_as_name() -> Result<()> {
        let zone = Zone::new("Europe/Paris")?;
        let json = serde_json::to_string(&zone).context("serialise")?;
        assert_eq!(json, "\"Europe/Paris\"");
        assert_eq!(
            serde_json::from_str::<Zone>(&json).context("deserialise")?,
            zone
        );
        assert!(serde_json::from_str::<Zone>("\"Mars/Olympus\"").is_err());
        Ok(())
    }

    #[test]
    fn test_weekday_next_wraps() {
        assert_eq!(Weekday::Sunday.next(), Weekday::Monday);
        assert_eq!(Weekday::Monday.index(), 0);
        assert_eq!(Weekday::Sunday.index(), 6);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_local_to_utc_round_trip_outside_gaps(day in 0_i64..3650, minute in 0_u16..1440) {
            let berlin = Zone::new("Europe/Berlin").map_err(|e| TestCaseError::fail(e.to_string()))?;
            let base = SimTime::from_millis(1_767_225_600_000 + day * 86_400_000).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let date = berlin.local(base).date();
            let utc = berlin.to_utc(date, minute).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let back = berlin.local(utc);
            // Inside the spring-forward gap the time moves forward by an hour; otherwise exact.
            if back.minute_of_day() == minute {
                prop_assert_eq!(back.date(), date);
            } else {
                prop_assert_eq!(back.minute_of_day(), minute + 60);
            }
        }
    }
}
