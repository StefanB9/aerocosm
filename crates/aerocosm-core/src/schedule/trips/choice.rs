//! Choices for one service: aircraft type, departure time and far-end ground
//! time.

use super::{DAY, PLACEMENT_ATTEMPTS, RANGE_MARGIN, RUNWAY_MARGIN, TripContext};
use crate::{
    AircraftType, Id, SimRng, WakeCategory,
    airline::{Airline, Network},
    schedule::{
        FlightKind,
        network::Service,
        timetable::{Profile, pick_weighted, sample_minute},
    },
    units::Meters,
};

/// Passenger wide-bodies only fly sectors at least this long…
const WIDE_BODY_MIN_METERS: f64 = 1_200_000.0;
/// …and regional aircraft at most this long, unless nothing else fits.
const REGIONAL_MAX_METERS: f64 = 1_500_000.0;
/// Point-to-point departures from this base-local time may stay overnight at
/// the far end.
const NIGHTSTOP_FROM: u32 = 19 * 60;
/// Probability that a late departure becomes a nightstop.
const NIGHTSTOP_CHANCE: f64 = 0.5;
/// Nightstop returns leave between 06:00 and 07:30 far-end local time.
const MORNING_RETURN: u32 = 6 * 60;
const MORNING_SPREAD: u32 = 90;
/// Hub flights leave this many minutes after a bank, plus up to
/// [`BANK_SPREAD`].
const BANK_OFFSET: u32 = 30;
const BANK_SPREAD: u32 = 30;
/// Extra ground time beyond the minimum turnaround, up to this many minutes.
const GROUND_SPREAD: u32 = 30;
/// One aircraft type per service, drawn from the airline's fleet mix among
/// types that can fly the sector; size rules (no short-haul wide-bodies, no
/// long-haul regionals) are relaxed only if nothing else fits.
pub(super) fn choose_aircraft(
    ctx: &TripContext<'_>,
    airline: &Airline,
    service: &Service,
    distance: Meters,
    rng: &mut SimRng,
) -> Option<Id<AircraftType>> {
    let shortest_runway = ctx
        .longest_runway(service.base)
        .value()
        .min(ctx.longest_runway(service.other).value());
    let options: Vec<(Id<AircraftType>, &AircraftType, f64)> = airline
        .fleet()
        .iter()
        .filter_map(|share| {
            let id = ctx.aircraft.by_code(share.aircraft)?;
            let aircraft = ctx.aircraft.get(id)?;
            let flies = aircraft.range().to_meters().value() >= distance.value() * RANGE_MARGIN
                && shortest_runway >= aircraft.min_runway_length().value() * RUNWAY_MARGIN;
            flies.then_some((id, aircraft, share.share))
        })
        .collect();
    let sized = |aircraft: &AircraftType| {
        let wide = !aircraft.is_freighter()
            && matches!(aircraft.wake(), WakeCategory::Heavy | WakeCategory::Super);
        let regional = !aircraft.is_freighter() && aircraft.seats() <= 100;
        let short_wide = wide && distance.value() < WIDE_BODY_MIN_METERS;
        let long_regional = regional && distance.value() > REGIONAL_MAX_METERS;
        !short_wide && !long_regional
    };
    let weights: Vec<f64> = options
        .iter()
        .map(|(_, aircraft, share)| if sized(aircraft) { *share } else { 0.0 })
        .collect();
    let index = pick_weighted(rng, &weights).or_else(|| {
        let shares: Vec<f64> = options.iter().map(|(_, _, share)| *share).collect();
        pick_weighted(rng, &shares)
    })?;
    Some(options[index].0)
}

/// The departure time from the base for a slot, before curfew checks. Cargo
/// flies at night (D39) but moves to the day after half the attempts, where a
/// curfew closes the night (as at Frankfurt).
pub(super) fn departure(
    ctx: &TripContext<'_>,
    airline: &Airline,
    service: &Service,
    attempt: u32,
    rng: &mut SimRng,
) -> u32 {
    if service.kind == FlightKind::Cargo && attempt < PLACEMENT_ATTEMPTS / 2 {
        return u32::from(sample_minute(rng, Profile::CargoNight));
    }
    if airline.network() == Network::Hub
        && let Some(banks) = ctx.banks.get(&service.base)
        && let Ok(index) = usize::try_from(
            rng.below(u32::try_from(banks.len()).unwrap_or(1))
                .unwrap_or(0),
        )
        && let Some(bank) = banks.get(index)
    {
        return (u32::from(bank.minute_of_day())
            + BANK_OFFSET
            + rng.below(BANK_SPREAD).unwrap_or(0))
            % DAY;
    }
    u32::from(sample_minute(rng, Profile::PassengerDay))
}

/// Ground time at the far end: a turnaround, or for some late point-to-point
/// departures a nightstop returning next morning far-end local time (D42).
pub(super) fn ground_time(
    ctx: &TripContext<'_>,
    airline: &Airline,
    service: &Service,
    depart: u32,
    out_block: u32,
    turnaround: u32,
    rng: &mut SimRng,
) -> u32 {
    let ground = turnaround + rng.below(GROUND_SPREAD + 1).unwrap_or(0);
    let nightstop = service.kind == FlightKind::Passenger
        && airline.network() != Network::Hub
        && depart >= NIGHTSTOP_FROM
        && rng.next_f64() < NIGHTSTOP_CHANCE;
    if !nightstop {
        return ground;
    }
    // Arrival in far-end local minutes on an absolute scale, then the next
    // 06:00-07:30.
    let arrive =
        i64::from(depart + out_block) - ctx.offset(service.base) + ctx.offset(service.other);
    let wanted = i64::from(MORNING_RETURN + rng.below(MORNING_SPREAD).unwrap_or(0));
    let mut morning = arrive.div_euclid(i64::from(DAY)) * i64::from(DAY) + wanted;
    while morning < arrive + i64::from(turnaround) {
        morning += i64::from(DAY);
    }
    u32::try_from(morning - arrive).unwrap_or(ground)
}
