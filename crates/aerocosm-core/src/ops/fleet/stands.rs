//! Stands at mapped airports (plan M4 D68, D69): which tail stands where,
//! taken on arrival or before the first departure, left at off-block.

use super::Operations;
use crate::{
    Id, LatLon,
    ground::{Node, Stand, StandRequest, TAXI_SPEED},
    ops::path::arrival_route,
    scenario::Scenario,
    world::{Airport, codes::AirlineIcao},
};

/// How many tails stand on each stand of each airport. A stand is normally
/// held by one tail; when nothing suitable is free an arrival shares one
/// until waiting spots arrive (plan M4 PR 3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct StandBook {
    /// By airport index, then stand index; empty for unmapped airports.
    taken: Vec<Vec<u16>>,
}

impl StandBook {
    /// No stand taken at any of `scenario`'s mapped airports.
    pub(super) fn new(scenario: &Scenario) -> Self {
        let world = scenario.world();
        let taken = world
            .airports()
            .map(|(id, _)| {
                scenario
                    .ground()
                    .airport(id)
                    .map_or_else(Vec::new, |layout| vec![0; layout.stands().len()])
            })
            .collect();
        Self { taken }
    }

    /// Whether `stand` at `airport` is free.
    pub(super) fn is_free(&self, airport: Id<Airport>, stand: Id<Stand>) -> bool {
        self.count(airport, stand) == 0
    }

    fn count(&self, airport: Id<Airport>, stand: Id<Stand>) -> u16 {
        self.taken
            .get(airport.as_usize())
            .and_then(|stands| stands.get(stand.as_usize()))
            .copied()
            .unwrap_or(0)
    }

    fn slot(&mut self, airport: Id<Airport>, stand: Id<Stand>) -> Option<&mut u16> {
        self.taken
            .get_mut(airport.as_usize())?
            .get_mut(stand.as_usize())
    }

    /// Marks `stand` taken by one more tail.
    pub(super) fn take(&mut self, airport: Id<Airport>, stand: Id<Stand>) {
        if let Some(count) = self.slot(airport, stand) {
            *count = count.saturating_add(1);
        }
    }

    /// Marks `stand` left by one tail.
    pub(super) fn release(&mut self, airport: Id<Airport>, stand: Id<Stand>) {
        if let Some(count) = self.slot(airport, stand) {
            *count = count.saturating_sub(1);
        }
    }

    /// Tails on stands at `airport`.
    #[cfg(test)]
    pub(super) fn occupied(&self, airport: Id<Airport>) -> u32 {
        self.taken
            .get(airport.as_usize())
            .map_or(0, |stands| stands.iter().map(|&n| u32::from(n)).sum())
    }
}

impl Operations {
    /// Puts tail `index` on the best stand at `airport` near `near` (and
    /// reachable from node `from`, if given), unless it already stands there;
    /// `None` at unmapped airports or when no stand fits its aircraft.
    pub(super) fn take_stand(
        &mut self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        airline: AirlineIcao,
        near: (LatLon, Option<Id<Node>>),
    ) -> Option<Id<Stand>> {
        self.find_stand(scenario, index, airport, airline, near, true)
    }

    /// Like [`Self::take_stand`]; with `shared` false only a free stand is
    /// taken, else none.
    pub(super) fn find_stand(
        &mut self,
        scenario: &Scenario,
        index: usize,
        airport: Id<Airport>,
        airline: AirlineIcao,
        near: (LatLon, Option<Id<Node>>),
        shared: bool,
    ) -> Option<Id<Stand>> {
        let (near, from) = near;
        let layout = scenario.ground().airport(airport)?;
        // One-way taxiways split some networks: only stands it can get to.
        let reach = from.map(|node| layout.reachable_from(node, self.wingspans[index]));
        let reachable = |stand: Id<Stand>| {
            reach.as_ref().is_none_or(|reach| {
                layout
                    .stands()
                    .get(stand.as_usize())
                    .is_some_and(|s| reach.get(s.node().as_usize()).copied().unwrap_or(false))
            })
        };
        if let Some((at, stand)) = self.parked[index]
            && at == airport
            && reachable(stand)
        {
            return Some(stand);
        }
        self.release_stand(index);
        let request = StandRequest {
            airline,
            wingspan: self.wingspans[index],
            near,
        };
        let areas = scenario.stand_areas().areas(airport);
        let book = &self.stands;
        let stand = layout
            .choose_stand(areas, &request, |stand| {
                book.is_free(airport, stand) && reachable(stand)
            })
            .or_else(|| {
                shared
                    .then(|| layout.choose_stand(areas, &request, reachable))
                    .flatten()
            })
            .or_else(|| {
                // Every stand that fits lies in other airlines' exclusive
                // areas (curated for the airlines of 2025, plan M4 D68): one
                // of them rather than waiting all day.
                shared
                    .then(|| layout.choose_stand(&[], &request, reachable))
                    .flatten()
            })
            .or_else(|| {
                // Too large for every stand (a small airfield's apron): the
                // largest there is rather than none.
                let largest = layout
                    .stands()
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| Id::from_usize(*i).is_ok_and(reachable))
                    .map(|(_, s)| s.max_wingspan())
                    .max_by(|a, b| a.value().total_cmp(&b.value()))?;
                let smaller = StandRequest {
                    wingspan: largest,
                    ..request
                };
                shared
                    .then(|| layout.choose_stand(areas, &smaller, reachable))
                    .flatten()
            });
        if let Some(stand) = stand {
            self.stands.take(airport, stand);
            self.parked[index] = Some((airport, stand));
        }
        stand
    }

    /// Takes tail `index` off its stand.
    pub(super) fn release_stand(&mut self, index: usize) {
        if let Some((airport, stand)) = self.parked[index].take() {
            self.stands.release(airport, stand);
        }
    }

    /// On touchdown at a mapped destination: a stand near the exit and the
    /// route to it. Returns the taxi-in time, s, for the tail's motion.
    pub(super) fn stand_on_arrival(
        &mut self,
        scenario: &Scenario,
        index: usize,
        destination: Id<Airport>,
        airline: AirlineIcao,
    ) -> Option<f64> {
        let (exit, near) = self.paths[index]
            .as_ref()
            .and_then(|path| path.exit().map(|exit| (exit, path.exit_point())))?;
        let stand = self.take_stand(scenario, index, destination, airline, (near, None))?;
        let layout = scenario.ground().airport(destination)?;
        let (route, heading) = arrival_route(layout, exit, stand, self.wingspans[index])?;
        let path = self.paths[index].as_mut()?;
        let length = path.set_taxi_in(route, heading);
        self.profiles[index].set_taxi_in_length(length);
        Some(length / TAXI_SPEED)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed, SimTime,
        ground::{GroundDefinition, StandAreasDefinition},
        ops::{Phase, StartMode},
        schedule::{ScheduleConfig, generate},
        test_data::data,
    };

    fn midday() -> Result<SimTime> {
        "2026-07-08T10:00:00Z".parse()
    }

    fn mapped(mode: StartMode, seed: u64) -> Result<(Scenario, Operations)> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?
                .with_ground(GroundDefinition::from_json(include_str!(
                    "../../../../../assets/ground/germany.ground.json"
                ))?)?
                .with_stand_areas(StandAreasDefinition::from_json(include_str!(
                    "../../../../../assets/ground/stand-areas.json"
                ))?)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), midday()?))?;
        let operations = Operations::new(&scenario, schedule, midday()?, RngSeed::new(seed), mode)?;
        Ok((scenario, operations))
    }

    fn frankfurt(scenario: &Scenario) -> Result<Id<Airport>> {
        scenario
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")
    }

    /// Stands holding more than one tail, and stands taken while a variant
    /// of them is.
    fn clashes(scenario: &Scenario, operations: &Operations, airport: Id<Airport>) -> usize {
        let Some(layout) = scenario.ground().airport(airport) else {
            return 0;
        };
        (0..layout.stands().len())
            .filter_map(|i| Id::<Stand>::from_usize(i).ok())
            .filter(|&stand| {
                let count = operations.stands.count(airport, stand);
                count > 1
                    || (count == 1
                        && layout
                            .variants(stand)
                            .iter()
                            .any(|&v| !operations.stands.is_free(airport, v)))
            })
            .count()
    }

    #[test]
    fn test_tails_on_the_ground_stand_on_real_stands() -> Result<()> {
        let (scenario, operations) = mapped(StartMode::Warm, 1)?;
        let fra = frankfurt(&scenario)?;
        let parked = operations.stands.occupied(fra);
        assert!(parked > 50, "{parked} tails on FRA stands");
        let clashing = clashes(&scenario, &operations, fra);
        assert!(
            clashing * 20 <= usize::try_from(parked)?,
            "{clashing} of {parked}"
        );
        let layout = scenario.ground().airport(fra).context("FRA layout")?;
        let mut boarding = 0;
        for view in operations.flights() {
            if view.origin() != fra || view.phase() != Phase::Boarding {
                continue;
            }
            let index = view.tail().as_usize();
            let (airport, stand) = operations.parked[index].context("a boarding tail stands")?;
            assert_eq!(airport, fra);
            let at = layout.nodes()[layout.stands()[stand.as_usize()].node().as_usize()];
            assert!(view.position().distance_to(at.position()).value() < 1.0);
            boarding += 1;
        }
        assert!(boarding > 5, "{boarding}");
        Ok(())
    }

    #[test]
    fn test_arrivals_taxi_to_the_stand_they_took() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Warm, 2)?;
        let fra = frankfurt(&scenario)?;
        let layout = scenario.ground().airport(fra).context("FRA layout")?;
        let mut checked = 0;
        for _ in 0..90 {
            for _ in 0..20 {
                operations.step(&scenario)?;
            }
            for view in operations.flights() {
                if view.destination() != fra || view.phase() != Phase::TaxiIn {
                    continue;
                }
                let index = view.tail().as_usize();
                let Some((airport, stand)) = operations.parked[index] else {
                    continue;
                };
                assert_eq!(airport, fra);
                let path = operations.paths[index].as_ref().context("a path")?;
                let motion = operations.tails[index].motion.context("moving")?;
                let mut done = motion;
                done.set_taxi_in(0.0);
                done.advance_free(&operations.profiles[index], 1.0);
                let (end, _) = path.locate(&done);
                let at = layout.nodes()[layout.stands()[stand.as_usize()].node().as_usize()];
                assert!(end.distance_to(at.position()).value() < 1.0);
                checked += 1;
            }
        }
        assert!(checked > 20, "{checked} taxi-ins checked");
        assert!(clashes(&scenario, &operations, fra) < 10);
        Ok(())
    }

    #[test]
    fn test_most_legs_from_mapped_airports_taxi_on_routes() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Warm, 4)?;
        let (mut routed, mut straight) = (0, 0);
        for _ in 0..12 {
            for _ in 0..300 {
                operations.step(&scenario)?;
            }
            for view in operations.flights() {
                if view.phase() != Phase::Boarding
                    || scenario.ground().airport(view.origin()).is_none()
                {
                    continue;
                }
                let path = operations.paths[view.tail().as_usize()]
                    .as_ref()
                    .context("a path")?;
                if path.routed_out() {
                    routed += 1;
                } else {
                    straight += 1;
                }
            }
        }
        assert!(routed > 100, "{routed}");
        assert!(
            straight * 20 <= routed,
            "{straight} straight, {routed} routed"
        );
        Ok(())
    }

    #[test]
    fn test_stands_are_deterministic() -> Result<()> {
        let (scenario, mut first) = mapped(StartMode::Cold, 3)?;
        let (_, mut second) = mapped(StartMode::Cold, 3)?;
        for _ in 0..600 {
            first.step(&scenario)?;
            second.step(&scenario)?;
        }
        assert_eq!(first.parked, second.parked);
        assert_eq!(first.stands, second.stands);
        Ok(())
    }

    #[test]
    fn test_aircraft_too_large_for_every_stand_take_the_largest() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Cold, 3)?;
        let airport = scenario
            .world()
            .airport_by_ident("EDVE".parse()?)
            .context("EDVE")?;
        let layout = scenario.ground().airport(airport).context("layout")?;
        let largest = layout
            .stands()
            .iter()
            .map(|s| s.max_wingspan().value())
            .fold(0.0, f64::max);
        let near = layout.nodes()[0].position();
        operations.release_stand(0);
        operations.wingspans[0] = crate::units::Meters::new(largest + 10.0)?;
        // No free stand fits…
        let airline = "KLM".parse()?;
        assert_eq!(
            operations.find_stand(&scenario, 0, airport, airline, (near, None), false),
            None
        );
        // …so a shared one is the largest there is.
        let stand = operations
            .take_stand(&scenario, 0, airport, airline, (near, None))
            .context("a stand")?;
        let span = layout.stands()[stand.as_usize()].max_wingspan().value();
        assert!((span - largest).abs() < 1e-9, "{span} of {largest}");
        Ok(())
    }

    #[test]
    fn test_stand_book_counts_tails() {
        let mut book = StandBook {
            taken: vec![vec![0; 3]],
        };
        let (airport, stand) = (Id::new(0), Id::new(1));
        assert!(book.is_free(airport, stand));
        book.take(airport, stand);
        book.take(airport, stand);
        assert!(!book.is_free(airport, stand));
        assert_eq!(book.occupied(airport), 2);
        book.release(airport, stand);
        book.release(airport, stand);
        book.release(airport, stand);
        assert!(book.is_free(airport, stand));
        assert!(book.is_free(Id::new(5), stand));
        book.take(Id::new(5), stand);
        assert_eq!(book.occupied(Id::new(5)), 0);
    }
}
