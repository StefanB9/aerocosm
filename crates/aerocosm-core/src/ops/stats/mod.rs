//! Delay statistics (plan M3 D62): punctuality, delays, holding and ground
//! delays per airport and for the network, and movements per clock hour.

mod causes;

pub use self::causes::{DelayCauses, LATE_AIRCRAFT_REFERENCES};
use crate::{Id, SimDuration, SimTime, world::Airport};

/// Flights within this delay count as on time.
const ON_TIME: SimDuration = SimDuration::from_minutes(15);
/// Milliseconds per clock hour…
const HOUR: i64 = 3_600_000;
/// …and per day.
const DAY: i64 = 24 * HOUR;

/// Statistics of one airport, or of the whole network.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AirportStats {
    departures: u32,
    on_time_departures: u32,
    departure_delay: i64,
    /// Departure delay by cause, totals (plan M8 D167).
    delay_causes: DelayCauses,
    /// Departures cancelled by their airline (plan M8 D171).
    cancellations: u32,
    /// Aircraft swapped onto a late trip here (plan M8 D169).
    swaps: u32,
    arrivals: u32,
    on_time_arrivals: u32,
    arrival_delay: i64,
    holds: u32,
    holding_seconds: f64,
    ground_delays: u32,
    ground_delay: i64,
    /// Take-offs plus landings per clock hour since the start.
    hours: Vec<u32>,
    /// Controlled taxis and their delay over unimpeded taxiing, ms (plan M4).
    taxis: u32,
    taxi_delay: i64,
    /// Time taxiing aircraft waited for a clearance, ms (plan M4 D88).
    clearance_wait: i64,
    /// The longest controlled taxi, from pushback or the runway to its
    /// stand or holding position, ms (plan M5 D144).
    longest_taxi: i64,
    /// Arrivals sent to a waiting spot for want of a stand.
    stand_waits: u32,
    /// Taxis that went ahead without a conflict-free plan (D75).
    ground_conflicts: u32,
    /// Taxis sent another way out of a deadlock (D93).
    reroutes: u32,
    /// Taxis that stood for minutes with nobody to wait for (plan M6 D150).
    stalls: u32,
    /// Approaches that went around for an occupied runway (D90).
    go_arounds: u32,
    /// Arrivals that shared a stand after waiting for one too long (D69).
    stand_shares: u32,
}

fn share(part: u32, whole: u32) -> f64 {
    if whole == 0 {
        0.0
    } else {
        f64::from(part) / f64::from(whole)
    }
}

fn average(total: i64, count: u32) -> SimDuration {
    if count == 0 {
        SimDuration::ZERO
    } else {
        SimDuration::from_millis(total / i64::from(count))
    }
}

impl AirportStats {
    /// Flights that left the gate.
    pub const fn departures(&self) -> u32 {
        self.departures
    }

    /// Share of departures within 15 minutes of schedule.
    pub fn on_time_departures(&self) -> f64 {
        share(self.on_time_departures, self.departures)
    }

    /// Mean departure delay.
    pub fn average_departure_delay(&self) -> SimDuration {
        average(self.departure_delay, self.departures)
    }

    /// Average departure delay by cause (plan M8 D167).
    pub fn departure_delay_causes(&self) -> DelayCauses {
        self.delay_causes.per(self.departures)
    }

    /// Departures cancelled by their airline (plan M8 D171).
    pub const fn cancellations(&self) -> u32 {
        self.cancellations
    }

    /// Aircraft swapped onto a late trip here (plan M8 D169).
    pub const fn swaps(&self) -> u32 {
        self.swaps
    }

    /// Share of scheduled departures cancelled: cancellations against
    /// departures flown plus cancelled.
    pub fn cancelled_share(&self) -> f64 {
        share(self.cancellations, self.departures + self.cancellations)
    }

    /// Flights that reached the gate.
    pub const fn arrivals(&self) -> u32 {
        self.arrivals
    }

    /// Share of arrivals within 15 minutes of schedule.
    pub fn on_time_arrivals(&self) -> f64 {
        share(self.on_time_arrivals, self.arrivals)
    }

    /// Mean arrival delay, early arrivals counting as none.
    pub fn average_arrival_delay(&self) -> SimDuration {
        average(self.arrival_delay, self.arrivals)
    }

    /// Holding patterns flown before landing here.
    pub const fn holds(&self) -> u32 {
        self.holds
    }

    /// Minutes spent holding before landing here.
    pub fn holding_minutes(&self) -> f64 {
        self.holding_seconds / 60.0
    }

    /// Flights held at their origin gate because of congestion here.
    pub const fn ground_delays(&self) -> u32 {
        self.ground_delays
    }

    /// Minutes of those ground delays.
    pub fn ground_delay_minutes(&self) -> f64 {
        // Totals stay far below 2^53 ms.
        #[allow(clippy::cast_precision_loss)]
        let minutes = self.ground_delay as f64 / 60_000.0;
        minutes
    }

    /// Taxis under ground control (plan M4).
    pub const fn taxis(&self) -> u32 {
        self.taxis
    }

    /// Average delay of a controlled taxi over taxiing unimpeded.
    pub fn average_taxi_delay(&self) -> SimDuration {
        average(self.taxi_delay, self.taxis)
    }

    /// Total time taxiing aircraft waited for a clearance, minutes (D88).
    pub fn clearance_wait_minutes(&self) -> f64 {
        // SAFETY: a run's waits stay far below 2^53 ms.
        #[allow(clippy::cast_precision_loss)]
        let millis = self.clearance_wait as f64;
        millis / 60_000.0
    }

    /// The longest controlled taxi that ended.
    pub fn longest_taxi(&self) -> SimDuration {
        SimDuration::from_millis(self.longest_taxi)
    }

    /// Average time a taxi waited for clearances.
    pub fn average_clearance_wait(&self) -> SimDuration {
        average(self.clearance_wait, self.taxis)
    }

    /// Arrivals that waited at a spot for a stand.
    pub const fn stand_waits(&self) -> u32 {
        self.stand_waits
    }

    /// Taxis that went ahead without a conflict-free plan (D75).
    pub const fn ground_conflicts(&self) -> u32 {
        self.ground_conflicts
    }

    /// Taxis sent another way out of a deadlock (D93).
    pub const fn reroutes(&self) -> u32 {
        self.reroutes
    }

    /// Taxis that stood for minutes with nobody to wait for: a fault of
    /// ground control, never expected (plan M6 D150).
    pub const fn stalls(&self) -> u32 {
        self.stalls
    }

    /// Approaches that went around for an occupied runway (D90).
    pub const fn go_arounds(&self) -> u32 {
        self.go_arounds
    }

    /// Arrivals that shared a stand after waiting for one too long (D69).
    pub const fn stand_shares(&self) -> u32 {
        self.stand_shares
    }

    /// Most take-offs plus landings in one clock hour.
    pub fn busiest_hour(&self) -> u32 {
        self.hours.iter().copied().max().unwrap_or(0)
    }

    fn add(&mut self, other: &Self) {
        self.departures += other.departures;
        self.on_time_departures += other.on_time_departures;
        self.departure_delay += other.departure_delay;
        self.delay_causes.add(other.delay_causes);
        self.cancellations += other.cancellations;
        self.swaps += other.swaps;
        self.arrivals += other.arrivals;
        self.on_time_arrivals += other.on_time_arrivals;
        self.arrival_delay += other.arrival_delay;
        self.holds += other.holds;
        self.holding_seconds += other.holding_seconds;
        self.ground_delays += other.ground_delays;
        self.ground_delay += other.ground_delay;
        self.taxis += other.taxis;
        self.taxi_delay += other.taxi_delay;
        self.clearance_wait += other.clearance_wait;
        self.longest_taxi = self.longest_taxi.max(other.longest_taxi);
        self.stand_waits += other.stand_waits;
        self.ground_conflicts += other.ground_conflicts;
        self.reroutes += other.reroutes;
        self.stalls += other.stalls;
        self.go_arounds += other.go_arounds;
        self.stand_shares += other.stand_shares;
    }
}

/// Departures of one day of a run, network-wide (plan M7 D163: delay must
/// not grow over the week).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayStats {
    departures: u32,
    on_time_departures: u32,
    departure_delay: i64,
    delay_causes: DelayCauses,
    cancellations: u32,
}

impl DayStats {
    /// Flights that left the gate that day.
    pub const fn departures(&self) -> u32 {
        self.departures
    }

    /// Share of them within 15 minutes of schedule.
    pub fn on_time_departures(&self) -> f64 {
        share(self.on_time_departures, self.departures)
    }

    /// Their average delay, early ones counting as none.
    pub fn average_departure_delay(&self) -> SimDuration {
        average(self.departure_delay, self.departures)
    }

    /// Departures cancelled that day (plan M8 D171).
    pub const fn cancellations(&self) -> u32 {
        self.cancellations
    }

    /// Their average delay by cause (plan M8 D167).
    pub fn departure_delay_causes(&self) -> DelayCauses {
        self.delay_causes.per(self.departures)
    }
}

/// Statistics of a run, per airport (by index) since its start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statistics {
    start: Option<SimTime>,
    airports: Vec<AirportStats>,
    /// Departures per day since the start.
    days: Vec<DayStats>,
}

impl Statistics {
    /// Empty statistics for a world of `airports` airports.
    pub(crate) fn new(airports: usize) -> Self {
        Self {
            start: None,
            airports: vec![AirportStats::default(); airports],
            days: Vec::new(),
        }
    }

    /// Counts hours from `start`.
    pub(crate) const fn set_start(&mut self, start: SimTime) {
        self.start = Some(start);
    }

    fn at(&mut self, airport: Id<Airport>) -> Option<&mut AirportStats> {
        self.airports.get_mut(airport.as_usize())
    }

    /// A departure from `airport` left the gate at `time`, late by
    /// `causes` after schedule.
    pub(crate) fn departed(&mut self, airport: Id<Airport>, causes: DelayCauses, time: SimTime) {
        let delay = causes.total();
        if let Some(today) = self.day(time) {
            today.departures += 1;
            today.on_time_departures += u32::from(delay <= ON_TIME);
            today.departure_delay += delay.as_millis().max(0);
            today.delay_causes.add(causes);
        }
        if let Some(at) = self.at(airport) {
            at.departures += 1;
            at.on_time_departures += u32::from(delay <= ON_TIME);
            at.departure_delay += delay.as_millis().max(0);
            at.delay_causes.add(causes);
        }
    }

    /// The airline cancelled a departure from `airport` scheduled at
    /// `scheduled` (plan M8 D171).
    pub(crate) fn cancelled(&mut self, airport: Id<Airport>, scheduled: SimTime) {
        if let Some(day) = self.day(scheduled) {
            day.cancellations += 1;
        }
        if let Some(at) = self.at(airport) {
            at.cancellations += 1;
        }
    }

    /// An airline swapped aircraft at `airport` onto a late trip.
    pub(crate) fn swapped(&mut self, airport: Id<Airport>) {
        if let Some(at) = self.at(airport) {
            at.swaps += 1;
        }
    }

    /// The day of a run `time` falls on, created if new; `None` before the
    /// start is set or before it.
    fn day(&mut self, time: SimTime) -> Option<&mut DayStats> {
        let start = self.start?;
        let day = usize::try_from(time.duration_since(start).as_millis() / DAY).ok()?;
        if self.days.len() <= day {
            self.days.resize(day + 1, DayStats::default());
        }
        self.days.get_mut(day)
    }

    /// Departures per day since the start, the first day first.
    pub fn days(&self) -> &[DayStats] {
        &self.days
    }

    /// An arrival reached its gate at `airport` `delay` after schedule.
    pub(crate) fn arrived(&mut self, airport: Id<Airport>, delay: SimDuration) {
        if let Some(at) = self.at(airport) {
            at.arrivals += 1;
            at.on_time_arrivals += u32::from(delay <= ON_TIME);
            at.arrival_delay += delay.as_millis().max(0);
        }
    }

    /// An arrival to `airport` left a hold after `seconds`.
    pub(crate) fn held(&mut self, airport: Id<Airport>, seconds: f64) {
        if let Some(at) = self.at(airport) {
            at.holds += 1;
            at.holding_seconds += seconds;
        }
    }

    /// A flight to `airport` was held at its origin gate for `delay`.
    pub(crate) fn regulated(&mut self, airport: Id<Airport>, delay: SimDuration) {
        if let Some(at) = self.at(airport) {
            at.ground_delays += 1;
            at.ground_delay += delay.as_millis().max(0);
        }
    }

    /// A controlled taxi at `airport` ended: its delay over taxiing
    /// unimpeded, and how long it took.
    pub(crate) fn taxied(&mut self, airport: Id<Airport>, delay: SimDuration, took: SimDuration) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.taxis += 1;
            stats.taxi_delay += delay.as_millis();
            stats.longest_taxi = stats.longest_taxi.max(took.as_millis());
        }
    }

    /// A taxiing aircraft at `airport` was cleared on after waiting `wait`.
    pub(crate) fn waited_for_clearance(&mut self, airport: Id<Airport>, wait: SimDuration) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.clearance_wait += wait.as_millis().max(0);
        }
    }

    /// Records an arrival sent to a waiting spot.
    pub(crate) fn waited_for_stand(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.stand_waits += 1;
        }
    }

    /// Records an arrival sharing a stand.
    pub(crate) fn shared_stand(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.stand_shares += 1;
        }
    }

    /// Records a taxi that went ahead without a conflict-free plan.
    pub(crate) fn ground_conflict(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.ground_conflicts += 1;
        }
    }

    /// Records a taxi standing with nobody to wait for.
    pub(crate) fn stalled(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.stalls += 1;
        }
    }

    /// Records a taxi sent another way out of a deadlock.
    pub(crate) fn rerouted(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.reroutes += 1;
        }
    }

    /// Records an approach that went around.
    pub(crate) fn went_around(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.go_arounds += 1;
        }
    }

    pub(crate) fn moved(&mut self, airport: Id<Airport>, time: SimTime) {
        let Some(start) = self.start else {
            return;
        };
        let hour = usize::try_from(time.duration_since(start).as_millis() / HOUR).unwrap_or(0);
        if let Some(at) = self.at(airport) {
            if at.hours.len() <= hour {
                at.hours.resize(hour + 1, 0);
            }
            at.hours[hour] += 1;
        }
    }

    /// Statistics of one airport; empty for an unknown one.
    pub fn airport(&self, airport: Id<Airport>) -> AirportStats {
        self.airports
            .get(airport.as_usize())
            .cloned()
            .unwrap_or_default()
    }

    /// Totals over all airports (busiest hour not included).
    pub fn network(&self) -> AirportStats {
        let mut total = AirportStats::default();
        for airport in &self.airports {
            total.add(airport);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn minutes(value: i32) -> SimDuration {
        SimDuration::from_minutes(value)
    }

    /// A departure `value` minutes late for flow control only.
    fn waited(value: i32) -> DelayCauses {
        DelayCauses::split(minutes(value), SimDuration::ZERO, SimDuration::ZERO)
    }

    #[test]
    fn test_delay_causes_per_airport_day_and_network() -> Result<()> {
        let mut stats = Statistics::new(2);
        let start: SimTime = "2026-07-06T00:00:00Z".parse()?;
        stats.set_start(start);
        let at = |hours: i32| start.checked_add(SimDuration::from_minutes(hours * 60));
        // 30 late: 20 for the aircraft, 10 more drawn (30 in all); 10 late:
        // all drawn.
        stats.departed(
            Id::new(0),
            DelayCauses::split(minutes(30), minutes(20), minutes(30)),
            at(1)?,
        );
        stats.departed(
            Id::new(0),
            DelayCauses::split(minutes(10), minutes(0), minutes(10)),
            at(2)?,
        );
        stats.departed(Id::new(1), waited(6), at(30)?);
        let airport = stats.airport(Id::new(0)).departure_delay_causes();
        assert_eq!(airport.late_aircraft(), minutes(10));
        assert_eq!(airport.airline(), minutes(10));
        assert_eq!(airport.flow_and_ground(), SimDuration::ZERO);
        assert_eq!(stats.days()[0].departure_delay_causes(), airport);
        let second = stats.days()[1].departure_delay_causes();
        assert_eq!(second.flow_and_ground(), minutes(6));
        let network = stats.network().departure_delay_causes();
        assert_eq!(network.total(), stats.network().average_departure_delay());
        assert!((network.late_aircraft_share() - 20.0 / 46.0).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn test_longest_taxi_per_airport_and_network() {
        let mut stats = Statistics::new(3);
        stats.taxied(Id::new(0), minutes(1), minutes(12));
        stats.taxied(Id::new(0), minutes(2), minutes(30));
        stats.taxied(Id::new(1), minutes(0), minutes(45));
        assert_eq!(stats.airport(Id::new(0)).longest_taxi(), minutes(30));
        assert_eq!(stats.airport(Id::new(1)).longest_taxi(), minutes(45));
        assert_eq!(stats.airport(Id::new(2)).longest_taxi(), SimDuration::ZERO);
        assert_eq!(stats.network().longest_taxi(), minutes(45));
    }

    #[test]
    fn test_ground_control_counts() {
        let mut stats = Statistics::new(2);
        let airport = Id::new(0);
        stats.taxied(airport, SimDuration::from_secs(60), minutes(5));
        stats.taxied(airport, SimDuration::from_secs(180), minutes(5));
        stats.waited_for_stand(airport);
        stats.ground_conflict(airport);
        stats.ground_conflict(Id::new(9));
        stats.shared_stand(airport);
        stats.shared_stand(airport);
        stats.rerouted(airport);
        stats.stalled(airport);
        stats.went_around(airport);
        stats.went_around(airport);
        let at = stats.airport(airport);
        assert_eq!(at.reroutes(), 1);
        assert_eq!((at.stalls(), stats.network().stalls()), (1, 1));
        assert_eq!(at.go_arounds(), 2);
        assert_eq!(stats.network().go_arounds(), 2);
        assert_eq!(at.taxis(), 2);
        assert_eq!(at.average_taxi_delay(), SimDuration::from_secs(120));
        assert_eq!(
            (at.stand_waits(), at.ground_conflicts(), at.stand_shares()),
            (1, 1, 2)
        );
        assert_eq!(stats.network().stand_shares(), 2);
        assert_eq!(stats.network().taxis(), 2);
        assert_eq!(
            stats.airport(Id::new(1)).average_taxi_delay(),
            SimDuration::ZERO
        );
    }

    #[test]
    fn test_clearance_waits_add_up_per_airport_and_taxi() {
        let mut stats = Statistics::new(2);
        let airport = Id::new(0);
        stats.taxied(airport, SimDuration::ZERO, minutes(5));
        stats.taxied(airport, SimDuration::ZERO, minutes(5));
        stats.waited_for_clearance(airport, SimDuration::from_secs(90));
        stats.waited_for_clearance(airport, SimDuration::from_secs(150));
        stats.waited_for_clearance(Id::new(9), SimDuration::from_secs(60));
        let at = stats.airport(airport);
        assert!((at.clearance_wait_minutes() - 4.0).abs() < 1e-9);
        assert_eq!(at.average_clearance_wait(), SimDuration::from_secs(120));
        assert!((stats.network().clearance_wait_minutes() - 4.0).abs() < 1e-9);
        assert_eq!(
            stats.airport(Id::new(1)).average_clearance_wait(),
            SimDuration::ZERO
        );
    }

    #[test]
    fn test_cancellations_per_airport_day_and_network() -> Result<()> {
        let mut stats = Statistics::new(2);
        let start: SimTime = "2026-07-06T00:00:00Z".parse()?;
        stats.set_start(start);
        let at = |hours: i32| start.checked_add(SimDuration::from_minutes(hours * 60));
        stats.departed(Id::new(0), waited(0), at(1)?);
        stats.departed(Id::new(0), waited(0), at(2)?);
        stats.departed(Id::new(1), waited(0), at(3)?);
        stats.cancelled(Id::new(0), at(4)?);
        stats.cancelled(Id::new(1), at(30)?);
        assert_eq!(stats.airport(Id::new(0)).cancellations(), 1);
        assert!((stats.airport(Id::new(0)).cancelled_share() - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(stats.days()[0].cancellations(), 1);
        assert_eq!(stats.days()[1].cancellations(), 1);
        assert_eq!(stats.days()[1].departures(), 0);
        let network = stats.network();
        assert_eq!(network.cancellations(), 2);
        assert!((network.cancelled_share() - 0.4).abs() < 1e-12);
        assert!(AirportStats::default().cancelled_share().abs() < 1e-12);
        stats.swapped(Id::new(1));
        stats.swapped(Id::new(5));
        assert_eq!(stats.airport(Id::new(1)).swaps(), 1);
        assert_eq!(stats.network().swaps(), 1);
        Ok(())
    }

    #[test]
    fn test_departures_are_counted_by_day() -> Result<()> {
        let mut stats = Statistics::new(1);
        let start: SimTime = "2026-07-06T00:00:00Z".parse()?;
        stats.set_start(start);
        let at = |hours: i32| start.checked_add(SimDuration::from_minutes(hours * 60));
        stats.departed(Id::new(0), waited(5), at(2)?);
        stats.departed(Id::new(0), waited(30), at(23)?);
        stats.departed(Id::new(0), waited(60), at(50)?);
        let days = stats.days();
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].departures(), 2);
        assert!((days[0].on_time_departures() - 0.5).abs() < 1e-12);
        assert_eq!(
            days[0].average_departure_delay(),
            minutes(17).checked_add(SimDuration::from_secs(30))?
        );
        assert_eq!(days[1].departures(), 0);
        assert_eq!(days[2].average_departure_delay(), minutes(60));
        Ok(())
    }

    #[test]
    fn test_punctuality_and_average_delays() {
        let mut stats = Statistics::new(3);
        let airport = Id::new(1);
        let start = SimTime::UNIX_EPOCH;
        stats.departed(airport, waited(0), start);
        stats.departed(airport, waited(10), start);
        stats.departed(airport, waited(40), start);
        stats.arrived(airport, minutes(-5));
        stats.arrived(airport, minutes(20));
        let at = stats.airport(airport);
        assert_eq!(at.departures(), 3);
        assert!((at.on_time_departures() - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(
            at.average_departure_delay(),
            SimDuration::from_millis(minutes(50).as_millis() / 3)
        );
        assert_eq!(at.arrivals(), 2);
        assert!((at.on_time_arrivals() - 0.5).abs() < 1e-12);
        // Early arrivals count as on time and add no delay.
        assert_eq!(at.average_arrival_delay(), minutes(10));
        let network = stats.network();
        assert_eq!((network.departures(), network.arrivals()), (3, 2));
    }

    #[test]
    fn test_holding_and_ground_delays() {
        let mut stats = Statistics::new(3);
        stats.held(Id::new(2), 600.0);
        stats.held(Id::new(2), 300.0);
        stats.regulated(Id::new(2), minutes(25));
        let at = stats.airport(Id::new(2));
        assert_eq!(at.holds(), 2);
        assert!((at.holding_minutes() - 15.0).abs() < 1e-9);
        assert_eq!(at.ground_delays(), 1);
        assert!((at.ground_delay_minutes() - 25.0).abs() < 1e-9);
        assert_eq!(stats.network().holds(), 2);
    }

    #[test]
    fn test_movements_per_clock_hour() -> Result<()> {
        let mut stats = Statistics::new(3);
        let start: SimTime = "2026-07-08T10:00:00Z".parse()?;
        stats.set_start(start);
        for minute in [5, 20, 59, 61, 125] {
            stats.moved(Id::new(0), start.checked_add(minutes(minute))?);
        }
        assert_eq!(stats.airport(Id::new(0)).busiest_hour(), 3);
        assert_eq!(stats.airport(Id::new(1)).busiest_hour(), 0);
        Ok(())
    }

    #[test]
    fn test_empty_statistics_read_as_zero() {
        let stats = Statistics::new(1);
        let at = stats.airport(Id::new(5));
        assert_eq!(at.departures(), 0);
        assert!(at.on_time_departures().abs() < 1e-12);
        assert_eq!(at.average_arrival_delay(), SimDuration::ZERO);
    }
}
