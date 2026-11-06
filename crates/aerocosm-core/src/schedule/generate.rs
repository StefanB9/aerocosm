//! Composing the stages: services → round trips → tails → flights.

use anyhow::{Context, Result};

use super::{
    Schedule, ScheduleConfig, ScheduleInputs, ScheduledFlight, Tail,
    network::plan_services,
    rotation::{Trip, WEEK, assign},
    trips::{DAY, TripContext, build_trips},
};
use crate::{Id, SimDuration, StreamId};

/// Generates the weekly schedule; the same inputs and config always give the
/// same schedule.
pub fn generate(inputs: &ScheduleInputs<'_>, config: ScheduleConfig) -> Result<Schedule> {
    let mut rng = config.seed().stream(StreamId::Scheduling);
    let services = plan_services(inputs.world, inputs.airlines, inputs.traffic)
        .context("cannot plan services")?;
    let ctx = TripContext::new(
        inputs.world,
        inputs.aircraft,
        inputs.airlines,
        inputs.traffic,
        config.reference(),
    )?;
    let outcome = build_trips(&ctx, &services, &mut rng).context("cannot build round trips")?;
    let trips: Vec<Trip<(u32, u32, u32)>> = outcome
        .trips
        .iter()
        .map(|trip| Trip {
            pool: (
                trip.airline.index(),
                trip.base.index(),
                trip.aircraft.index(),
            ),
            start: trip.start,
            end: trip.end(),
            turnaround: trip.base_turnaround,
        })
        .collect();
    let assignment = assign(&trips);

    let tails = assignment
        .tail_pools
        .iter()
        .map(|(airline, base, aircraft)| {
            let base = Id::new(*base);
            Ok(Tail {
                airline: Id::new(*airline),
                aircraft: Id::new(*aircraft),
                base,
                utc_offset_minutes: i32::try_from(ctx.offset(base)).context("UTC offset")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut order: Vec<usize> = (0..outcome.trips.len()).collect();
    order.sort_by_key(|index| (assignment.tails[*index], outcome.trips[*index].start));
    let mut flights = Vec::with_capacity(outcome.trips.len() * 2);
    let mut tail_starts = Vec::with_capacity(tails.len() + 1);
    let mut movements = vec![[0_u32; 7]; inputs.world.airport_count()];
    for index in order {
        let tail = assignment.tails[index];
        while tail_starts.len() <= tail {
            tail_starts.push(flights.len());
        }
        let trip = &outcome.trips[index];
        let tail_id = Id::from_usize(tail).context("tail index")?;
        let [depart, arrive, turn, back] = trip.events();
        for ((origin, departure), (destination, arrival), number) in [
            (depart, arrive, trip.numbers.0),
            (turn, back, trip.numbers.1),
        ] {
            flights.push(ScheduledFlight {
                airline: trip.airline,
                number,
                aircraft: trip.aircraft,
                tail: tail_id,
                origin,
                destination,
                departure: SimDuration::from_minutes(
                    i32::try_from(departure).context("departure minute")?,
                ),
                block: SimDuration::from_minutes(
                    i32::try_from(arrival - departure).context("block minutes")?,
                ),
                kind: trip.kind,
            });
        }
        for (airport, minute) in trip.events() {
            let local = i64::from(minute) - ctx.offset(trip.base) + ctx.offset(airport);
            let day = usize::try_from(local.rem_euclid(i64::from(WEEK)) / i64::from(DAY))
                .context("weekday")?;
            if let Some(counts) = movements.get_mut(airport.as_usize()) {
                counts[day] += 1;
            }
        }
    }
    tail_starts.push(flights.len());
    Ok(Schedule {
        tails,
        flights,
        tail_starts,
        movements,
        dropped_slots: outcome.dropped,
    })
}
