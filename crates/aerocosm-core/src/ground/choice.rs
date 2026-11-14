//! Generic stand choice (plan M5 D120): the tightest fitting stand, near the
//! stands the same airline holds, near where the aircraft comes from; cargo
//! airlines on the aprons mapped for cargo.

use super::{AirportGround, Stand};
use crate::{Id, LatLon, units::Meters};

/// Unused wingspan of a stand is compared in steps of this many metres, so
/// small aircraft do not take large stands while small ones are free.
const WASTE_STEP: f64 = 15.0;
/// Distances to the airline's other stands are compared in steps of this
/// many metres: an apron's width, so that the airline gathers on one.
const CLUSTER_STEP: f64 = 250.0;

/// How a stand ranks for a request, lowest first: on the wrong apron, unused
/// wingspan steps, distance steps to the airline's stands, distance to where
/// the aircraft comes from.
type Rank = (bool, u32, u32, f64);

/// What a stand is wanted for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandRequest {
    /// The aircraft's wingspan.
    pub wingspan: Meters,
    /// Where the aircraft comes from (e.g. its runway exit); nearer stands
    /// are preferred.
    pub near: LatLon,
    /// Whether the airline flies cargo.
    pub cargo: bool,
}

impl AirportGround {
    /// The best stand for `request` among those `free` reports free, with
    /// their variants: one on the apron for its kind of traffic (where any
    /// is mapped for cargo), then the tightest fit, then the nearest to a
    /// stand `mate` reports held by the same airline, then the nearest to
    /// where it comes from. `None` if nothing fits.
    pub fn choose_stand(
        &self,
        request: &StandRequest,
        mate: impl Fn(Id<Stand>) -> bool,
        free: impl Fn(Id<Stand>) -> bool,
    ) -> Option<Id<Stand>> {
        let stands = self.stands();
        let position = |stand: &Stand| self.nodes()[stand.node().as_usize()].position();
        let mates: Vec<LatLon> = stands
            .iter()
            .enumerate()
            .filter(|(i, _)| Id::from_usize(*i).is_ok_and(&mate))
            .map(|(_, stand)| position(stand))
            .collect();
        let mut best: Option<(Rank, Id<Stand>)> = None;
        for (index, stand) in stands.iter().enumerate() {
            let Ok(id) = Id::<Stand>::from_usize(index) else {
                continue;
            };
            if stand.max_wingspan() < request.wingspan
                || !free(id)
                || !self.variants(id).iter().all(|&variant| free(variant))
            {
                continue;
            }
            let at = position(stand);
            // SAFETY: small non-negative numbers of steps.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let waste = ((stand.max_wingspan() - request.wingspan).value() / WASTE_STEP) as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cluster = mates
                .iter()
                .map(|m| m.distance_to(at).value())
                .reduce(f64::min)
                .map_or(0, |d| (d / CLUSTER_STEP) as u32);
            let key = (
                stand.cargo() != request.cargo && self.has_cargo_stands(),
                waste,
                cluster,
                at.distance_to(request.near).value(),
            );
            if best.is_none_or(|(least, _)| key < least) {
                best = Some((key, id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Whether any stand lies on an apron mapped for cargo.
    fn has_cargo_stands(&self) -> bool {
        self.stands().iter().any(Stand::cargo)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::{ground::definition::tests::sample, test_data::data, units::Degrees};

    fn ground(stands: &[(&str, [f64; 2], f64, bool)]) -> Result<AirportGround> {
        let mut definition = sample()?;
        let template = definition.stands[0].clone();
        definition.stands.clear();
        for (name, point, span, cargo) in stands {
            definition.nodes.push(*point);
            let mut stand = template.clone();
            stand.name = name.parse()?;
            stand.node = u32::try_from(definition.nodes.len() - 1)?;
            stand.max_wingspan = Meters::new(*span)?;
            stand.heading = Degrees::new(0.0)?;
            stand.cargo = *cargo;
            definition.stands.push(stand);
        }
        AirportGround::new(definition, &data()?.world)
    }

    fn request(span: f64, cargo: bool) -> Result<StandRequest> {
        Ok(StandRequest {
            wingspan: Meters::new(span)?,
            near: LatLon::new(50.03, 8.56)?,
            cargo,
        })
    }

    fn name(ground: &AirportGround, stand: Option<Id<Stand>>) -> Option<String> {
        stand.and_then(|id| {
            ground
                .stands()
                .get(id.as_usize())
                .map(|s| s.name().to_string())
        })
    }

    const NONE: fn(Id<Stand>) -> bool = |_| false;
    const ALL: fn(Id<Stand>) -> bool = |_| true;

    #[test]
    fn test_choose_stand_prefers_fit_then_distance() -> Result<()> {
        let ground = ground(&[
            ("A1", [50.0350, 8.5700], 80.0, false),
            ("A2", [50.0360, 8.5700], 40.0, false),
            ("A3", [50.0370, 8.5700], 40.0, false),
        ])?;
        // An A320: the tight stand, the nearer of the two.
        let narrow = ground.choose_stand(&request(35.8, false)?, NONE, ALL);
        assert_eq!(name(&ground, narrow), Some("A2".to_owned()));
        // A widebody needs A1; an A380 there with A1 taken gets nothing.
        let wide = ground.choose_stand(&request(64.8, false)?, NONE, ALL);
        assert_eq!(name(&ground, wide), Some("A1".to_owned()));
        let a1_taken = |id: Id<Stand>| id.index() != 0;
        assert_eq!(
            ground.choose_stand(&request(79.8, false)?, NONE, a1_taken),
            None
        );
        Ok(())
    }

    #[test]
    fn test_choose_stand_gathers_an_airline_after_fit() -> Result<()> {
        let ground = ground(&[
            ("A1", [50.0301, 8.5601], 40.0, false),
            ("B1", [50.0400, 8.5900], 40.0, false),
            ("B2", [50.0401, 8.5901], 40.0, false),
            ("B3", [50.0402, 8.5902], 80.0, false),
        ])?;
        // The airline holds B1: B2 beside it, though A1 is nearer the exit.
        let b1_held = |id: Id<Stand>| id.index() == 1;
        let free = |id: Id<Stand>| id.index() != 1;
        let chosen = ground.choose_stand(&request(35.8, false)?, b1_held, free);
        assert_eq!(name(&ground, chosen), Some("B2".to_owned()));
        // With B2 taken too, fit beats gathering: A1, not the wide B3.
        let more = |id: Id<Stand>| id.index() != 1 && id.index() != 2;
        let chosen = ground.choose_stand(&request(35.8, false)?, b1_held, more);
        assert_eq!(name(&ground, chosen), Some("A1".to_owned()));
        Ok(())
    }

    #[test]
    fn test_choose_stand_keeps_cargo_on_cargo_aprons() -> Result<()> {
        let ground = ground(&[
            ("A1", [50.0301, 8.5601], 80.0, false),
            ("F1", [50.0500, 8.6000], 80.0, true),
        ])?;
        let cargo = ground.choose_stand(&request(64.8, true)?, NONE, ALL);
        assert_eq!(name(&ground, cargo), Some("F1".to_owned()));
        let passenger = ground.choose_stand(&request(64.8, false)?, NONE, ALL);
        assert_eq!(name(&ground, passenger), Some("A1".to_owned()));
        // The other apron rather than none.
        let a1_taken = |id: Id<Stand>| id.index() != 0;
        let overflow = ground.choose_stand(&request(64.8, false)?, NONE, a1_taken);
        assert_eq!(name(&ground, overflow), Some("F1".to_owned()));
        Ok(())
    }

    #[test]
    fn test_choose_stand_skips_stands_whose_variant_is_taken() -> Result<()> {
        let ground = ground(&[
            ("05", [50.0300, 8.5600], 60.0, false),
            ("05A", [50.0300, 8.5602], 40.0, false),
            ("06", [50.0400, 8.5800], 40.0, false),
        ])?;
        assert_eq!(ground.variants(Id::new(1)), &[Id::new(0)]);
        let wide_parked = |id: Id<Stand>| id.index() != 0;
        let chosen = ground.choose_stand(&request(35.8, false)?, NONE, wide_parked);
        assert_eq!(name(&ground, chosen), Some("06".to_owned()));
        Ok(())
    }
}
