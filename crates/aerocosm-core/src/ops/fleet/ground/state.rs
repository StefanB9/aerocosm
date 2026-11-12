//! What ground control is doing with each aircraft, for the flight view
//! (plan M4 D85).

use crate::ops::{
    Callsign, Phase, TaxiGoal, TaxiRoute, TaxiState, fleet::Operations, taxi::via_names,
};

/// Below this ground speed an aircraft counts as holding, m/s.
const HOLDING_SPEED: f64 = 0.5;

impl Operations {
    /// Tail `index`'s taxi state in `phase`, moving at `speed` m/s; `None`
    /// off the ground of a controlled airport.
    pub(in crate::ops::fleet) fn taxi_state(
        &self,
        index: usize,
        phase: Phase,
        speed: f64,
    ) -> Option<TaxiState> {
        let taxi = self.taxis.get(index)?.as_ref()?;
        let to = taxi.goal;
        match phase {
            Phase::Pushback => return Some(TaxiState::Pushback { stand: taxi.stand }),
            Phase::Queue => {
                return match to {
                    TaxiGoal::HoldingPoint(runway) => Some(TaxiState::AtHoldingPoint { runway }),
                    _ => None,
                };
            }
            Phase::TaxiOut | Phase::TaxiIn => {}
            _ => return None,
        }
        if taxi.escaped {
            return Some(TaxiState::ClearedRegardless { to });
        }
        if taxi.waiting.is_some() {
            return Some(TaxiState::WaitingForStand);
        }
        let airside = self.ground.airside(taxi.airport)?;
        let along = self.along_of(index).unwrap_or(taxi.offset);
        let names = || {
            let mut walked = taxi.offset;
            taxi.edges.iter().filter_map(move |&(edge, _)| {
                walked += airside.lengths.get(edge.as_usize()).copied().unwrap_or(0.0);
                (walked > along)
                    .then(|| airside.names.get(edge.as_usize()).copied())
                    .flatten()
            })
        };
        let holding = speed < HOLDING_SPEED;
        if holding && taxi.crossing.is_some() {
            let runway = names()
                .find(|&(_, runway)| runway)
                .and_then(|(name, _)| name);
            return Some(TaxiState::HoldingShort { runway });
        }
        if holding && let Some(flight) = taxi.waiting_for.and_then(|b| self.callsign(b as usize)) {
            return Some(TaxiState::WaitingFor { flight, to });
        }
        if let Some(flight) = taxi.leader.and_then(|l| self.callsign(l as usize)) {
            return Some(TaxiState::Following { flight, to });
        }
        Some(TaxiState::Taxiing {
            via: via_names(names().map(|(name, _)| name)),
            to,
        })
    }

    /// Tail `index`'s taxi route while it pushes back or taxies (D86).
    pub(crate) fn taxi_route(&self, index: usize) -> Option<TaxiRoute> {
        let taxi = self.taxis.get(index)?.as_ref()?;
        let motion = self.tails.get(index)?.motion.as_ref()?;
        if !matches!(
            motion.phase(),
            Phase::Pushback | Phase::TaxiOut | Phase::TaxiIn
        ) {
            return None;
        }
        TaxiRoute::new(taxi.points.clone(), motion.along(), taxi.limit(), taxi.end)
    }

    /// The callsign tail `index` flies under now.
    pub(in crate::ops::fleet) fn callsign(&self, index: usize) -> Option<Callsign> {
        let tail = self.tails.get(index)?;
        let flight = self.flight(index, tail.leg).ok()?;
        Some(Callsign::new(
            *self.callsigns.get(flight.airline().as_usize())?,
            flight.number(),
        ))
    }
}
