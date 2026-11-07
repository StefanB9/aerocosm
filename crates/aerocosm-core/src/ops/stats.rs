//! Delay statistics (plan M3 D62): punctuality, delays, holding and ground
//! delays per airport and for the network, and movements per clock hour.

use crate::{Id, SimDuration, SimTime, world::Airport};

/// Flights within this delay count as on time.
const ON_TIME: SimDuration = SimDuration::from_minutes(15);
/// Milliseconds per clock hour.
const HOUR: i64 = 3_600_000;

/// Statistics of one airport, or of the whole network.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AirportStats {
    departures: u32,
    on_time_departures: u32,
    departure_delay: i64,
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
    /// Arrivals sent to a waiting spot for want of a stand.
    stand_waits: u32,
    /// Taxis that went ahead without a conflict-free plan (D75).
    ground_conflicts: u32,
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

    /// Arrivals that waited at a spot for a stand.
    pub const fn stand_waits(&self) -> u32 {
        self.stand_waits
    }

    /// Taxis that went ahead without a conflict-free plan (D75).
    pub const fn ground_conflicts(&self) -> u32 {
        self.ground_conflicts
    }

    /// Most take-offs plus landings in one clock hour.
    pub fn busiest_hour(&self) -> u32 {
        self.hours.iter().copied().max().unwrap_or(0)
    }

    fn add(&mut self, other: &Self) {
        self.departures += other.departures;
        self.on_time_departures += other.on_time_departures;
        self.departure_delay += other.departure_delay;
        self.arrivals += other.arrivals;
        self.on_time_arrivals += other.on_time_arrivals;
        self.arrival_delay += other.arrival_delay;
        self.holds += other.holds;
        self.holding_seconds += other.holding_seconds;
        self.ground_delays += other.ground_delays;
        self.ground_delay += other.ground_delay;
        self.taxis += other.taxis;
        self.taxi_delay += other.taxi_delay;
        self.stand_waits += other.stand_waits;
        self.ground_conflicts += other.ground_conflicts;
    }
}

/// Statistics of a run, per airport (by index) since its start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statistics {
    start: Option<SimTime>,
    airports: Vec<AirportStats>,
}

impl Statistics {
    /// Empty statistics for a world of `airports` airports.
    pub(crate) fn new(airports: usize) -> Self {
        Self {
            start: None,
            airports: vec![AirportStats::default(); airports],
        }
    }

    /// Counts hours from `start`.
    pub(crate) const fn set_start(&mut self, start: SimTime) {
        self.start = Some(start);
    }

    fn at(&mut self, airport: Id<Airport>) -> Option<&mut AirportStats> {
        self.airports.get_mut(airport.as_usize())
    }

    /// A departure from `airport` left the gate `delay` after schedule.
    pub(crate) fn departed(&mut self, airport: Id<Airport>, delay: SimDuration) {
        if let Some(at) = self.at(airport) {
            at.departures += 1;
            at.on_time_departures += u32::from(delay <= ON_TIME);
            at.departure_delay += delay.as_millis().max(0);
        }
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

    /// A take-off or landing at `airport` at `time`.
    pub(crate) fn taxied(&mut self, airport: Id<Airport>, delay: SimDuration) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.taxis += 1;
            stats.taxi_delay += delay.as_millis();
        }
    }

    /// Records an arrival sent to a waiting spot.
    pub(crate) fn waited_for_stand(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.stand_waits += 1;
        }
    }

    /// Records a taxi that went ahead without a conflict-free plan.
    pub(crate) fn ground_conflict(&mut self, airport: Id<Airport>) {
        if let Some(stats) = self.airports.get_mut(airport.as_usize()) {
            stats.ground_conflicts += 1;
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

    #[test]
    fn test_ground_control_counts() {
        let mut stats = Statistics::new(2);
        let airport = Id::new(0);
        stats.taxied(airport, SimDuration::from_secs(60));
        stats.taxied(airport, SimDuration::from_secs(180));
        stats.waited_for_stand(airport);
        stats.ground_conflict(airport);
        stats.ground_conflict(Id::new(9));
        let at = stats.airport(airport);
        assert_eq!(at.taxis(), 2);
        assert_eq!(at.average_taxi_delay(), SimDuration::from_secs(120));
        assert_eq!((at.stand_waits(), at.ground_conflicts()), (1, 1));
        assert_eq!(stats.network().taxis(), 2);
        assert_eq!(
            stats.airport(Id::new(1)).average_taxi_delay(),
            SimDuration::ZERO
        );
    }

    #[test]
    fn test_punctuality_and_average_delays() {
        let mut stats = Statistics::new(3);
        let airport = Id::new(1);
        stats.departed(airport, minutes(0));
        stats.departed(airport, minutes(10));
        stats.departed(airport, minutes(40));
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
