//! What ground control is doing with each aircraft, for the flight view
//! (plan M4 D85).

use crate::ops::{Callsign, Phase, TaxiGoal, TaxiState, fleet::Operations, taxi::via_names};

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
        let names = |from: usize| {
            taxi.hops
                .get(from..)
                .unwrap_or_default()
                .iter()
                .filter_map(|hop| hop.via)
                .filter_map(|(edge, _)| airside.names.get(edge.as_usize()).copied())
        };
        let holding = speed < HOLDING_SPEED;
        if holding && taxi.crossing.is_some() {
            let runway = names(taxi.granted)
                .find(|&(_, runway)| runway)
                .and_then(|(name, _)| name);
            return Some(TaxiState::HoldingShort { runway });
        }
        if holding && let Some(flight) = taxi.waiting_for.and_then(|b| self.callsign(b as usize)) {
            return Some(TaxiState::WaitingFor { flight, to });
        }
        Some(TaxiState::Taxiing {
            via: via_names(names(taxi.passed).map(|(name, _)| name)),
            to,
        })
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
