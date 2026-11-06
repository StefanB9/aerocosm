//! Lookups for trip building: distances, runways, zones, curfews, banks and
//! slot books.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::{DAY, RoundTrip};
use crate::{
    AircraftCatalog, Id, SimTime,
    airline::AirlineCatalog,
    ground::TaxiTimes,
    schedule::{
        rotation::WEEK,
        slots::SlotBook,
        timetable::{TaxiMinutes, offset_minutes},
    },
    traffic::{Capacity, ClockTime, Curfew, TrafficTargets},
    units::Meters,
    world::{Airport, World},
};

/// Lookups the trip builder needs, with zone offsets taken at a reference
/// instant (D43).
pub(crate) struct TripContext<'a> {
    world: &'a World,
    pub(super) aircraft: &'a AircraftCatalog,
    pub(super) airlines: &'a AirlineCatalog,
    pub(super) weekday_factors: [f64; 7],
    offsets: HashMap<Id<Airport>, i64>,
    curfews: HashMap<Id<Airport>, Curfew>,
    pub(super) banks: HashMap<Id<Airport>, Vec<ClockTime>>,
    capacities: HashMap<Id<Airport>, Capacity>,
    /// Typical taxi minutes at mapped airports (plan M4 D73).
    taxi: HashMap<Id<Airport>, (f64, f64)>,
}

impl<'a> TripContext<'a> {
    /// Prepares offsets, curfews and banks for every airport with a known zone.
    pub(crate) fn new(
        world: &'a World,
        aircraft: &'a AircraftCatalog,
        airlines: &'a AirlineCatalog,
        traffic: &TrafficTargets,
        reference: SimTime,
    ) -> Result<Self> {
        let mut offsets = HashMap::new();
        for (id, airport) in world.airports() {
            if let Some(zone) = traffic.zone(airport.ident()) {
                offsets.insert(id, i64::from(offset_minutes(zone, reference)));
            }
        }
        let mut curfews = HashMap::new();
        let mut banks = HashMap::new();
        let mut capacities = HashMap::new();
        for target in traffic.airports() {
            let id = world
                .airport_by_ident(target.airport())
                .with_context(|| format!("target airport {} not in the world", target.airport()))?;
            if let Some(curfew) = target.curfew() {
                curfews.insert(id, curfew);
            }
            if !target.banks().is_empty() {
                banks.insert(id, target.banks().to_vec());
            }
            if let Some(capacity) = target.capacity() {
                capacities.insert(id, capacity);
            }
        }
        Ok(Self {
            world,
            aircraft,
            airlines,
            weekday_factors: *traffic.weekday_factors(),
            offsets,
            curfews,
            banks,
            capacities,
            taxi: HashMap::new(),
        })
    }

    /// The context with typical taxi times by airport index (plan M4 D73).
    pub(crate) fn with_taxi(mut self, table: &[Option<TaxiTimes>]) -> Self {
        let minutes = |d: crate::SimDuration| d.as_secs_f64() / 60.0;
        for (index, times) in table.iter().enumerate() {
            if let (Some(times), Ok(id)) = (times, Id::<Airport>::from_usize(index)) {
                self.taxi
                    .insert(id, (minutes(times.out), minutes(times.back)));
            }
        }
        self
    }

    /// Taxi minutes a block from `from` to `to` allows.
    pub(crate) fn taxi(&self, from: Id<Airport>, to: Id<Airport>) -> TaxiMinutes {
        TaxiMinutes {
            out: self
                .taxi
                .get(&from)
                .map_or(TaxiMinutes::DEFAULT.out, |t| t.0),
            back: self
                .taxi
                .get(&to)
                .map_or(TaxiMinutes::DEFAULT.back, |t| t.1),
        }
    }

    /// Converts a base-local week minute to the local week minute at
    /// `airport`, wrapped into the week.
    pub(crate) fn local_week_minute(
        &self,
        base: Id<Airport>,
        airport: Id<Airport>,
        minute: u32,
    ) -> u32 {
        let local = i64::from(minute) - self.offset(base) + self.offset(airport);
        // SAFETY: rem_euclid(WEEK) lies in 0..WEEK, which fits u32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wrapped = local.rem_euclid(i64::from(WEEK)) as u32;
        wrapped
    }

    /// Empty slot books for every coordinated airport.
    pub(super) fn slot_books(&self) -> HashMap<Id<Airport>, SlotBook> {
        self.capacities
            .iter()
            .map(|(airport, capacity)| (*airport, SlotBook::new(*capacity)))
            .collect()
    }

    pub(super) fn curfew_free(&self, trip: &RoundTrip) -> bool {
        trip.events().iter().all(|(airport, minute)| {
            self.curfew(*airport)
                .is_none_or(|c| !c.contains(self.local_minute(trip.base, *airport, *minute)))
        })
    }

    /// Local week minutes of the trip's movements at `airport` on each of
    /// `days`.
    fn movements_at(&self, trip: &RoundTrip, airport: Id<Airport>, days: &[u32]) -> Vec<u32> {
        let mut minutes = Vec::with_capacity(days.len() * 2);
        for (at, minute) in trip.events() {
            if at == airport {
                minutes.extend(
                    days.iter()
                        .map(|day| self.local_week_minute(trip.base, airport, minute + day * DAY)),
                );
            }
        }
        minutes
    }

    /// Whether the trip fits the slot books at both ends on all `days`.
    pub(super) fn coordinated(
        &self,
        books: &HashMap<Id<Airport>, SlotBook>,
        trip: &RoundTrip,
        days: &[u32],
    ) -> bool {
        [trip.base, trip.other].iter().all(|airport| {
            books
                .get(airport)
                .is_none_or(|book| book.fits(&self.movements_at(trip, *airport, days)))
        })
    }

    /// Books the trip's movements at both ends on all `days`.
    pub(super) fn book(
        &self,
        books: &mut HashMap<Id<Airport>, SlotBook>,
        trip: &RoundTrip,
        days: &[u32],
    ) {
        for airport in [trip.base, trip.other] {
            let minutes = self.movements_at(trip, airport, days);
            if let Some(book) = books.get_mut(&airport) {
                book.book(&minutes);
            }
        }
    }

    fn airport(&self, id: Id<Airport>) -> Result<&'a Airport> {
        self.world.airport(id).context("airport id out of range")
    }

    /// Great-circle distance between two airports.
    pub(crate) fn distance(&self, a: Id<Airport>, b: Id<Airport>) -> Result<Meters> {
        Ok(self
            .airport(a)?
            .location()
            .distance_to(self.airport(b)?.location()))
    }

    /// Length of the airport's longest runway; zero without runways.
    pub(crate) fn longest_runway(&self, airport: Id<Airport>) -> Meters {
        self.world
            .runways_of(airport)
            .iter()
            .map(crate::world::Runway::length)
            .fold(Meters::default(), |a, b| if b > a { b } else { a })
    }

    /// The airport's night curfew, if any.
    pub(crate) fn curfew(&self, airport: Id<Airport>) -> Option<Curfew> {
        self.curfews.get(&airport).copied()
    }

    /// The airport's UTC offset at the reference instant, in minutes; zero
    /// without a known zone.
    pub(in crate::schedule) fn offset(&self, airport: Id<Airport>) -> i64 {
        self.offsets.get(&airport).copied().unwrap_or(0)
    }

    /// Converts a base-local week minute to local time of day at `airport`.
    pub(crate) fn local_minute(
        &self,
        base: Id<Airport>,
        airport: Id<Airport>,
        minute: u32,
    ) -> ClockTime {
        let local = i64::from(minute) - self.offset(base) + self.offset(airport);
        // SAFETY: rem_euclid(1440) is in 0..1440, which fits u16 and is a valid
        // clock time.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minute_of_day = local.rem_euclid(i64::from(DAY)) as u16;
        ClockTime::from_minute(minute_of_day).unwrap_or(ClockTime::MIDNIGHT)
    }
}
