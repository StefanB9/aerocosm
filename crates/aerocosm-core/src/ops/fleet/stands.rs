//! Stands at mapped airports (plan M4 D68, D69): which tail stands where,
//! taken on arrival or before the first departure, left at off-block.

use super::Operations;
use crate::{
    Id, LatLon,
    ground::{AirportGround, Node, Stand, StandRequest, TAXI_SPEED},
    ops::path::arrival_route,
    scenario::Scenario,
    units::Meters,
    world::{Airport, codes::AirlineIcao},
};

/// How many tails stand on each stand of each airport. A stand is normally
/// held by one tail; when nothing suitable is free an arrival shares one
/// until waiting spots arrive (plan M4 PR 3).
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct StandBook {
    /// By airport index, then stand index; empty for unmapped airports.
    taken: Vec<Vec<u16>>,
    /// The airline of the first tail on each stand, likewise (plan M5 D120).
    holders: Vec<Vec<Option<AirlineIcao>>>,
    /// The wingspan of the largest tail on each stand, m, likewise; zero on
    /// a free stand (plan M6 D148).
    spans: Vec<Vec<f32>>,
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
            .collect::<Vec<Vec<u16>>>();
        let holders = taken
            .iter()
            .map(|stands| vec![None; stands.len()])
            .collect();
        let spans = taken.iter().map(|stands| vec![0.0; stands.len()]).collect();
        Self {
            taken,
            holders,
            spans,
        }
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

    /// Whether a tail of `airline` holds `stand` at `airport`.
    pub(super) fn held_by(
        &self,
        airport: Id<Airport>,
        stand: Id<Stand>,
        airline: AirlineIcao,
    ) -> bool {
        self.holders
            .get(airport.as_usize())
            .and_then(|stands| stands.get(stand.as_usize()))
            .is_some_and(|holder| *holder == Some(airline))
    }

    fn holder(
        &mut self,
        airport: Id<Airport>,
        stand: Id<Stand>,
    ) -> Option<&mut Option<AirlineIcao>> {
        self.holders
            .get_mut(airport.as_usize())?
            .get_mut(stand.as_usize())
    }

    /// Whether an aircraft of `wingspan` has room on `stand` of `layout`: the
    /// stand is free and the aircraft on the stands beside it leave it the
    /// wingtip clearance (plan M6 D148).
    pub(super) fn has_room(
        &self,
        layout: &AirportGround,
        stand: Id<Stand>,
        wingspan: Meters,
    ) -> bool {
        let airport = layout.airport();
        self.is_free(airport, stand)
            && layout.neighbours(stand).iter().all(|&other| {
                self.span_on(airport, other)
                    .is_none_or(|span| layout.neighbours_clear((stand, wingspan), (other, span)))
            })
    }

    /// The wingspan of the largest tail on `stand`, if any stands there.
    fn span_on(&self, airport: Id<Airport>, stand: Id<Stand>) -> Option<Meters> {
        let span = *self.spans.get(airport.as_usize())?.get(stand.as_usize())?;
        (span > 0.0).then(|| Meters::from_raw(f64::from(span)))
    }

    /// Marks `stand` taken by one more tail, of `airline` and `wingspan`.
    pub(super) fn take(
        &mut self,
        airport: Id<Airport>,
        stand: Id<Stand>,
        airline: AirlineIcao,
        wingspan: Meters,
    ) {
        // SAFETY: wingspans are tens of metres.
        #[allow(clippy::cast_possible_truncation)]
        if let Some(span) = self
            .spans
            .get_mut(airport.as_usize())
            .and_then(|stands| stands.get_mut(stand.as_usize()))
        {
            *span = span.max(wingspan.value() as f32);
        }
        let Some(count) = self.slot(airport, stand) else {
            return;
        };
        *count = count.saturating_add(1);
        if *count == 1
            && let Some(holder) = self.holder(airport, stand)
        {
            *holder = Some(airline);
        }
    }

    /// Marks `stand` left by one tail.
    pub(super) fn release(&mut self, airport: Id<Airport>, stand: Id<Stand>) {
        let Some(count) = self.slot(airport, stand) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count > 0 {
            return;
        }
        if let Some(holder) = self.holder(airport, stand) {
            *holder = None;
        }
        if let Some(span) = self
            .spans
            .get_mut(airport.as_usize())
            .and_then(|stands| stands.get_mut(stand.as_usize()))
        {
            *span = 0.0;
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
            wingspan: self.wingspans[index],
            near,
            cargo: self.cargo[index],
        };
        let book = &self.stands;
        let mate = |stand| book.held_by(airport, stand, airline);
        let stand = layout
            .choose_stand(&request, mate, |stand| {
                book.has_room(layout, stand, request.wingspan) && reachable(stand)
            })
            .or_else(|| {
                shared
                    .then(|| layout.choose_stand(&request, mate, reachable))
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
                    .then(|| layout.choose_stand(&smaller, mate, reachable))
                    .flatten()
            });
        if let Some(stand) = stand {
            // Taken while another tail or a variant holds it: shared (D69).
            let held = !self.stands.has_room(layout, stand, request.wingspan)
                || layout
                    .variants(stand)
                    .iter()
                    .any(|&variant| !self.stands.is_free(airport, variant));
            if held {
                self.stats.shared_stand(airport);
            }
            self.stands.take(airport, stand, airline, request.wingspan);
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
pub(super) mod tests {
    use anyhow::{Context, Result};

    use super::*;
    use crate::{
        RngSeed, SimTime,
        ops::{Phase, StartMode},
        schedule::{ScheduleConfig, generate},
        test_data::data,
    };

    fn midday() -> Result<SimTime> {
        "2026-07-08T10:00:00Z".parse()
    }

    pub(in crate::ops::fleet) fn mapped(
        mode: StartMode,
        seed: u64,
    ) -> Result<(Scenario, Operations)> {
        let data = data()?;
        let scenario =
            Scenario::with_traffic(data.world, data.aircraft, data.airlines, data.traffic)?
                .with_ground(crate::test_data::ground()?)?;
        let inputs = scenario.schedule_inputs().context("traffic")?;
        let schedule = generate(&inputs, ScheduleConfig::new(RngSeed::new(1), midday()?))?;
        let operations = Operations::new(&scenario, schedule, midday()?, RngSeed::new(seed), mode)?;
        Ok((scenario, operations))
    }

    #[test]
    fn test_survey_lists_are_emptied_wherever_written() -> Result<()> {
        // Pushbacks approved after the survey write to their airport's
        // lists: those airports must be emptied before the next survey too.
        let (scenario, mut operations) = mapped(StartMode::Warm, 1)?;
        for _ in 0..300 {
            operations.step(&scenario)?;
            assert_eq!(
                operations.ground.unmarked_with_entries(),
                Vec::<usize>::new(),
                "at {}",
                operations.time
            );
        }
        Ok(())
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
                let span = operations.stands.span_on(airport, stand);
                count > 1
                    || (count == 1
                        && layout
                            .variants(stand)
                            .iter()
                            .any(|&v| !operations.stands.is_free(airport, v)))
                    || span.is_some_and(|span| {
                        layout.neighbours(stand).iter().any(|&n| {
                            operations.stands.span_on(airport, n).is_some_and(|other| {
                                !layout.stands_clear((stand, span), (n, other))
                            })
                        })
                    })
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
            let index = view.airframe().as_usize();
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
                let index = view.airframe().as_usize();
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
        let clashing = clashes(&scenario, &operations, fra);
        assert!(clashing < 10, "{clashing} clashes");
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
                let path = operations.paths[view.airframe().as_usize()]
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
        // Any mapped airport: the aircraft is made larger than its stands.
        let layout = scenario.ground().airports().first().context("a layout")?;
        let airport = layout.airport();
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
    fn test_long_stays_wait_in_remote_parking() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Warm, 5)?;
        let fra = frankfurt(&scenario)?;
        let layout = scenario.ground().airport(fra).context("FRA layout")?;
        for _ in 0..3_600 {
            operations.step(&scenario)?;
        }
        let at_fra = |index: usize| -> Result<bool> {
            let leg = operations.tails[index].leg;
            Ok(operations.flight(index, leg)?.origin() == fra)
        };
        let mut remote = 0;
        for index in 0..operations.tails.len() {
            if operations.is_spare(index)
                || operations.tails[index].motion.is_some()
                || !at_fra(index)?
            {
                continue;
            }
            if let super::super::remote::Parking::Remote(_) = operations.parking[index] {
                // Off its stand, and not boarding within the hour.
                assert_eq!(operations.parked[index], None);
                remote += 1;
            }
        }
        assert!(remote > 20, "{remote} tails in remote parking at FRA");
        // The stands hold what boards and turns round: no more tails than
        // stands, nearly no stand shared.
        let parked = operations.stands.occupied(fra);
        assert!(
            usize::try_from(parked)? <= layout.stands().len(),
            "{parked}"
        );
        assert!(clashes(&scenario, &operations, fra) <= 2);
        Ok(())
    }

    #[test]
    fn test_without_free_stands_aircraft_board_and_deplane_remote() -> Result<()> {
        let (scenario, mut operations) = mapped(StartMode::Warm, 6)?;
        let fra = frankfurt(&scenario)?;
        let layout = scenario.ground().airport(fra).context("FRA layout")?;
        // Every stand blocked by an aircraft that never leaves.
        let blocker = "ZZZ".parse()?;
        for i in 0..layout.stands().len() {
            let limit = layout.stands()[i].max_wingspan();
            operations
                .stands
                .take(fra, Id::from_usize(i)?, blocker, limit);
        }
        let mut boarded = std::collections::HashSet::new();
        let mut deplaned = std::collections::HashSet::new();
        for _ in 0..(150 * 60) {
            operations.step(&scenario)?;
            for index in 0..operations.tails.len() {
                match operations.parking[index] {
                    super::super::remote::Parking::Spot(_) => {
                        boarded.insert(index);
                    }
                    super::super::remote::Parking::Remote(None)
                        if operations.tails[index].motion.is_none()
                            && operations
                                .flight(index, operations.tails[index].leg)
                                .is_ok_and(|f| f.origin() == fra) =>
                    {
                        deplaned.insert(index);
                    }
                    _ => {}
                }
            }
        }
        let stats = operations.stats.airport(fra);
        let (boarded, deplaned) = (boarded.len(), deplaned.len());
        assert!(boarded > 0, "nobody boarded at a spot");
        assert!(deplaned > 0, "nobody deplaned remote");
        // Only arrivals that find neither a stand nor a spot free share one.
        assert!(
            usize::try_from(stats.stand_shares())? * 4 <= boarded + deplaned,
            "{boarded} boarded, {deplaned} deplaned, {} shares",
            stats.stand_shares()
        );
        Ok(())
    }

    #[test]
    fn test_stand_book_counts_tails_and_their_airline() -> Result<()> {
        let mut book = StandBook {
            taken: vec![vec![0; 3]],
            holders: vec![vec![None; 3]],
            spans: vec![vec![0.0; 3]],
        };
        let span = Meters::new(35.8)?;
        let (airport, stand) = (Id::new(0), Id::new(1));
        let (dlh, afr) = ("DLH".parse()?, "AFR".parse()?);
        assert!(book.is_free(airport, stand));
        assert!(!book.held_by(airport, stand, dlh));
        book.take(airport, stand, dlh, span);
        book.take(airport, stand, afr, span);
        assert!(!book.is_free(airport, stand));
        assert_eq!(book.occupied(airport), 2);
        // The first tail's airline holds it while anyone stands there.
        assert!(book.held_by(airport, stand, dlh) && !book.held_by(airport, stand, afr));
        book.release(airport, stand);
        assert!(book.held_by(airport, stand, dlh));
        book.release(airport, stand);
        book.release(airport, stand);
        assert!(book.is_free(airport, stand));
        assert!(!book.held_by(airport, stand, dlh));
        assert!(book.is_free(Id::new(5), stand));
        book.take(Id::new(5), stand, dlh, span);
        assert_eq!(book.occupied(Id::new(5)), 0);
        Ok(())
    }

    #[test]
    fn test_a_stand_has_room_beside_aircraft_that_leave_it_room() -> Result<()> {
        let (scenario, _) = mapped(StartMode::Cold, 3)?;
        let fra = frankfurt(&scenario)?;
        let layout = scenario.ground().airport(fra).context("FRA layout")?;
        let mut book = StandBook::new(&scenario);
        let dlh = "DLH".parse()?;
        let wide = crate::units::Meters::new(64.8)?;
        // A stand with a neighbour a wide-body on it would be too close to.
        let (stand, beside) = (0..layout.stands().len())
            .filter_map(|i| Id::<Stand>::from_usize(i).ok())
            .find_map(|stand| {
                layout
                    .neighbours(stand)
                    .iter()
                    .copied()
                    .find(|&n| !layout.stands_clear((stand, wide), (n, wide)))
                    .map(|n| (stand, n))
            })
            .context("two stands close together")?;
        assert!(book.has_room(layout, stand, wide));
        book.take(fra, beside, dlh, wide);
        assert!(!book.has_room(layout, beside, wide), "taken itself");
        for span in [24.0, 36.0, 52.0, 64.8] {
            let span = crate::units::Meters::new(span)?;
            assert_eq!(
                book.has_room(layout, stand, span),
                layout.stands_clear((stand, span), (beside, wide)),
                "{span:?}"
            );
        }
        book.release(fra, beside);
        assert!(book.has_room(layout, stand, wide));
        Ok(())
    }
}
